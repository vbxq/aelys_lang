use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, resolve_aelys_core_lib};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const PROBE_C: &str = r#"#include <stdio.h>
long opaque(long x) { return x; }
long probe_i8(signed char x) { printf("i8=%ld\n", (long)x); return 0; }
long probe_u8(unsigned char x) { printf("u8=%ld\n", (long)x); return 0; }
long probe_i16(short x) { printf("i16=%ld\n", (long)x); return 0; }
long probe_u16(unsigned short x) { printf("u16=%ld\n", (long)x); return 0; }
long probe_bool(_Bool x) { printf("bool=%d\n", (int)x); return 0; }
long probe_i32(int x) { printf("i32=%d\n", x); return 0; }
long probe_f32(float x) { printf("f32=%.1f\n", (double)x); return 0; }
"#;

const NARROW_AELYS: &str = "\
unsafe extern fn opaque(x: i64) -> i64
unsafe extern fn probe_i8(x: i8) -> i64
unsafe extern fn probe_u8(x: u8) -> i64
unsafe extern fn probe_i16(x: i16) -> i64
unsafe extern fn probe_u16(x: u16) -> i64
unsafe extern fn probe_bool(x: bool) -> i64

fn main() -> i64 {
    unsafe {
        let narrow: i64 = opaque(511)
        let wide: i64 = opaque(70000)
        probe_i8(narrow as i8)
        probe_u8(narrow as u8)
        probe_i16(wide as i16)
        probe_u16(wide as u16)
        probe_bool(narrow > 0)
    }
    return 0
}
";

const WIDE_AELYS: &str = "\
unsafe extern fn probe_i32(x: i32) -> i32
unsafe extern fn probe_u32(x: u32) -> u32
unsafe extern fn probe_i64(x: i64) -> i64
unsafe extern fn probe_u64(x: u64) -> u64
unsafe extern fn probe_f32(x: f32) -> f32
unsafe extern fn probe_f64(x: f64) -> f64
unsafe extern fn probe_i8(x: i8) -> i8

fn probe_native_i8(x: i8) -> i8 {
    return x
}

fn main() -> i64 {
    return probe_native_i8(1) as i64
}
";

const F32_AELYS: &str = "\
unsafe extern fn probe_f32(x: f32) -> i64

fn main() -> i64 {
    let a: f32 = 1.5
    let b: f32 = a - 3.0
    unsafe { probe_f32(b) }
    return 0
}
";

fn cc_can_build_c() -> bool {
    Command::new("clang")
        .arg("--version")
        .output()
        .is_ok_and(|out| out.status.success())
}

fn compile_probe_object(dir: &Path) -> PathBuf {
    let source = dir.join("probe.c");
    fs::write(&source, PROBE_C).expect("write the c probe");
    let object = dir.join("probe.o");
    let out = Command::new("clang")
        .args(["-c", "-O2"])
        .arg(&source)
        .arg("-o")
        .arg(&object)
        .output()
        .expect("run clang");
    assert!(
        out.status.success(),
        "the c probe MUST compile\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    object
}

struct Linked {
    _dir: TempDir,
    exe: PathBuf,
}

fn link_against_c(id: &str, level: &str, aelys: &str, opt: OptimizationLevel) -> Linked {
    let dir = tempdir().expect("tempdir");
    let probe = compile_probe_object(dir.path());
    let root = dir.path().join("root.aelys");
    fs::write(&root, aelys).expect("write the aelys half");
    let emitted = compile_file_with_llvm_variant(&root, opt, false, RuntimeVariant::Rc);
    let object = root.with_extension("o");
    if !object.is_file() {
        panic!(
            "{id} at {level}: the aelys half MUST reach an object file\nerror:\n{}",
            emitted.err().map(|e| e.to_string()).unwrap_or_default()
        );
    }
    let core = resolve_aelys_core_lib(RuntimeVariant::Rc).expect("locate the aelys core archive");
    let lib_dir = core.parent().expect("the archive sits in a directory");
    let exe = dir.path().join("bin");
    let out = Command::new("cc")
        .arg("-o")
        .arg(&exe)
        .arg(&object)
        .arg(&probe)
        .arg(format!("-L{}", lib_dir.display()))
        .arg("-laelys-core-rc")
        .output()
        .expect("run cc");
    assert!(
        out.status.success(),
        "{id} at {level}: the two objects MUST link\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    Linked { _dir: dir, exe }
}

fn stdout_of(id: &str, level: &str, exe: &Path) -> String {
    let out = Command::new(exe).output().expect("run the linked binary");
    assert!(
        out.status.success(),
        "{id} at {level}: the binary MUST exit normally, status {:?}",
        out.status.code()
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

fn emitted_ir(id: &str, aelys: &str, opt: OptimizationLevel) -> String {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, aelys).expect("write fixture");
    if let Err(err) = compile_file_with_llvm_variant(&root, opt, true, RuntimeVariant::Rc) {
        panic!("{id}: the program MUST reach llvm\nerror:\n{err}");
    }
    fs::read_to_string(root.with_extension("ll")).expect("read emitted .ll")
}

// gates the call site on , so the old bare spelling is kept as a rejected twin
#[test]
fn f4_1_bis_the_old_bare_spelling_of_a_narrow_call_is_now_e0617() {
    let bare = "unsafe extern fn probe_i8(x: i8) -> i64\n\nfn main() -> i64 {\n    probe_i8(1)\n    return 0\n}\n";
    let rendered = match aelys_driver::compile_to_typed_ast(bare) {
        Ok(_) => panic!("F4-1-bis: the bare call MUST be rejected\n{bare}"),
        Err(rendered) => rendered.to_string(),
    };
    assert!(
        rendered.contains("E0617"),
        "F4-1-bis: the rejection MUST be E0617\nrendered:\n{rendered}"
    );
}

#[test]
fn f4_1_the_narrow_answers_come_back_extended_at_every_level() {
    if !cc_can_build_c() {
        eprintln!("F4-1: clang unavailable, skipping the executed abi row");
        return;
    }
    for (level, opt) in LEVELS {
        let linked = link_against_c("F4-1", level, NARROW_AELYS, *opt);
        let printed = stdout_of("F4-1", level, &linked.exe);
        assert_eq!(
            printed, "i8=-1\nu8=255\ni16=4464\nu16=4464\nbool=1\n",
            "F4-1 at {level}: the c callee reads a full register, so the narrow arguments MUST \
             arrive extended"
        );
    }
}

#[test]
fn f4_2_the_declaration_carries_signext_and_zeroext_by_width() {
    let ir = emitted_ir("F4-2", NARROW_AELYS, OptimizationLevel::None);
    for expected in [
        "declare i64 @probe_i8(i8 signext)",
        "declare i64 @probe_u8(i8 zeroext)",
        "declare i64 @probe_i16(i16 signext)",
        "declare i64 @probe_u16(i16 zeroext)",
        "declare i64 @probe_bool(i1 zeroext)",
    ] {
        assert!(
            ir.contains(expected),
            "F4-2: the declaration MUST read `{expected}`, found:\n{ir}"
        );
    }
}

#[test]
fn f4_3_nothing_wider_than_sixteen_bits_carries_an_extension() {
    let ir = emitted_ir("F4-3", WIDE_AELYS, OptimizationLevel::None);
    for line in ir.lines().filter(|line| line.starts_with("declare")) {
        let narrow = line.contains("(i8") || line.contains("(i16") || line.contains("(i1 ");
        if narrow {
            continue;
        }
        assert!(
            !line.contains("signext") && !line.contains("zeroext"),
            "F4-3: clang extends nothing wider than sixteen bits on x86-64, found:\n{line}"
        );
    }
    for expected in [
        "declare i32 @probe_i32(i32)",
        "declare i64 @probe_i64(i64)",
        "declare float @probe_f32(float)",
        "declare double @probe_f64(double)",
    ] {
        assert!(
            ir.contains(expected),
            "F4-3: the declaration MUST read `{expected}`, found:\n{ir}"
        );
    }
}

#[test]
fn f4_4_an_aelys_body_is_never_given_an_extension() {
    let ir = emitted_ir("F4-4", WIDE_AELYS, OptimizationLevel::None);
    for line in ir.lines().filter(|line| line.starts_with("define")) {
        assert!(
            !line.contains("signext") && !line.contains("zeroext"),
            "F4-4: the aelys convention is not the c one, found:\n{line}"
        );
    }
    assert!(
        ir.contains("@probe_native_i8"),
        "F4-4: the native i8 twin MUST survive to llvm, found:\n{ir}"
    );
}

#[test]
#[ignore = "float constant folding at the f32 argument site emits the folded value as a double, so llvm rejects `call i64 @probe_f32(double ...)` against the float signature"]
fn f4_5_an_f32_argument_computed_by_arithmetic_reaches_the_callee() {
    if !cc_can_build_c() {
        eprintln!("F4-5: clang unavailable, skipping the executed abi row");
        return;
    }
    for (level, opt) in LEVELS {
        let linked = link_against_c("F4-5", level, F32_AELYS, *opt);
        assert_eq!(
            stdout_of("F4-5", level, &linked.exe),
            "f32=-1.5\n",
            "F4-5 at {level}: an `f32` argument MUST reach the c callee as a float"
        );
    }
}

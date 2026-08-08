use aelys_driver::{RuntimeVariant, compile_file_with_llvm, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
}

fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found") || error.contains("failed to run")
}

fn run_with_stats(src: &str, variant: RuntimeVariant) -> Option<(i32, String)> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(&source_path, OptimizationLevel::None, false, variant) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                eprintln!("linker unavailable; skipping exec assertion");
                return None;
            }
            panic!("compilation/link should succeed: {err}");
        }
    }

    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        eprintln!("executable not produced (linker unavailable); skipping");
        return None;
    }

    let output = Command::new(&exe)
        .env("AELYS_RC_STATS", "1")
        .output()
        .expect("run compiled exe");
    let code = output.status.code().expect("exit code");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Some((code, stderr))
}

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let after = line.split("allocs=").nth(1)?;
    let allocs: i64 = after.split_whitespace().next()?.parse().ok()?;
    let frees: i64 = line.split("frees=").nth(1)?.trim().parse().ok()?;
    Some((allocs, frees))
}

fn rc_ir(src: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect("llvm backend compilation should succeed");
    fs::read_to_string(source_path.with_extension("ll")).expect("ir file")
}

fn find_core_archive(file: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, file: &str) -> Option<PathBuf> {
        let entries = fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = walk(&path, file) {
                    return Some(found);
                }
            } else if path.file_name().and_then(|s| s.to_str()) == Some(file) {
                return Some(path);
            }
        }
        None
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().unwrap_or(manifest);
    walk(&root.join("target"), file)
}

fn asan_run_cycles(src: &str) -> Option<(i32, String)> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(
        &source_path,
        OptimizationLevel::None,
        false,
        RuntimeVariant::RcCycles,
    ) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                eprintln!("linker unavailable; skipping ASan probe");
                return None;
            }
            panic!("compilation should succeed: {err}");
        }
    }

    let object = source_path.with_extension(if cfg!(windows) { "obj" } else { "o" });
    if !object.is_file() {
        eprintln!("object not produced; skipping ASan probe");
        return None;
    }
    let archive = find_core_archive("libaelys-core-rc-cycles.a")?;
    let lib_dir = archive.parent().expect("archive has a parent");

    let asan_exe = dir.path().join("module_asan");
    let link = Command::new("clang")
        .arg("-fsanitize=address")
        .arg("-g")
        .arg(&object)
        .arg(format!("-L{}", lib_dir.display()))
        .arg("-laelys-core-rc-cycles")
        .arg("-o")
        .arg(&asan_exe)
        .output();
    let link = match link {
        Ok(out) => out,
        Err(_) => {
            eprintln!("clang unavailable; skipping ASan probe");
            return None;
        }
    };
    if !link.status.success() {
        eprintln!(
            "ASan link failed; skipping:\n{}",
            String::from_utf8_lossy(&link.stderr)
        );
        return None;
    }

    let run = Command::new(&asan_exe)
        .env("AELYS_RC_STATS", "1")
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .output()
        .expect("run ASan-instrumented exe");
    let code = run.status.code().expect("exit code");
    let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
    Some((code, stderr))
}

const CYCLE_SRC: &str = r#"
struct Node { next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { next: Rc::null() })
    a.next = b
    b.next = a
    __aelys_collect()
    return 0
}
"#;

#[test]
fn c1_cycle_collected_under_rc_cycles_leaks_under_rc() {
    let Some((code_rc, stderr_rc)) = run_with_stats(CYCLE_SRC, RuntimeVariant::Rc) else {
        return;
    };
    assert_eq!(code_rc, 0, "rc run must exit 0; stderr:\n{stderr_rc}");
    let (allocs_rc, frees_rc) =
        parse_stats(&stderr_rc).unwrap_or_else(|| panic!("no stats under rc:\n{stderr_rc}"));
    assert_eq!(
        allocs_rc, 2,
        "the cycle allocates two Nodes; stderr:\n{stderr_rc}"
    );
    assert!(
        frees_rc < allocs_rc,
        "under pure rc the cycle must leak (frees<allocs); got allocs={allocs_rc} frees={frees_rc}"
    );

    let Some((code_cyc, stderr_cyc)) = run_with_stats(CYCLE_SRC, RuntimeVariant::RcCycles) else {
        return;
    };
    assert_eq!(
        code_cyc, 0,
        "rc+cycles run must exit 0; stderr:\n{stderr_cyc}"
    );
    let (allocs_cyc, frees_cyc) = parse_stats(&stderr_cyc)
        .unwrap_or_else(|| panic!("no stats under rc+cycles:\n{stderr_cyc}"));
    assert_eq!(
        (allocs_cyc, frees_cyc),
        (2, 2),
        "under rc+cycles the cycle must be collected (allocs==frees); stderr:\n{stderr_cyc}"
    );
}

#[test]
fn c1_cycle_construction_is_null_safe_under_leak() {
    let Some((code, stderr)) = run_with_stats(CYCLE_SRC, RuntimeVariant::Leak) else {
        return;
    };
    assert_eq!(
        code, 0,
        "cycle construction under leak must not crash; stderr:\n{stderr}"
    );
    let (a, f) = parse_stats(&stderr).unwrap_or_else(|| panic!("no stats:\n{stderr}"));
    assert_eq!((a, f), (2, 0), "leak never frees (2/0); stderr:\n{stderr}");
}

#[test]
fn c2_asan_clean_on_collected_cycle() {
    let Some((code, stderr)) = asan_run_cycles(CYCLE_SRC) else {
        return;
    };
    assert_eq!(
        code, 0,
        "ASan run must exit 0 (no sanitizer abort); stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "ASan must report no errors on the collected cycle; stderr:\n{stderr}"
    );
    let (a, f) = parse_stats(&stderr).unwrap_or_else(|| panic!("no stats under ASan:\n{stderr}"));
    assert_eq!(
        (a, f),
        (2, 2),
        "collected cycle under ASan must be 2/2; stderr:\n{stderr}"
    );
}

const C3_SRC: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 10, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 20, next: Rc::null() })
    a.next = b
    b.next = a
    let keep: Rc<Node> = a
    __aelys_collect()
    let n: Rc<Node> = keep.next
    let v: i64 = n.val
    return v
}
"#;

#[test]
fn c3_cycle_attached_to_live_root_survives_collect() {
    let Some((code, stderr)) = run_with_stats(C3_SRC, RuntimeVariant::RcCycles) else {
        return;
    };
    assert_eq!(
        code, 20,
        "anti-UAF: keep.next.val must read 20 after collect (A,B survived); stderr:\n{stderr}"
    );
    let (a, f) = parse_stats(&stderr).unwrap_or_else(|| panic!("no stats:\n{stderr}"));
    assert_eq!(
        (a, f),
        (2, 2),
        "end-of-main reclaims the cycle (2/2); stderr:\n{stderr}"
    );
}

#[test]
fn c3_asan_clean_with_live_root_and_postcollect_deref() {
    let Some((code, stderr)) = asan_run_cycles(C3_SRC) else {
        return;
    };
    assert_eq!(
        code, 20,
        "ASan anti-UAF run must exit 20; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "ASan must report no errors on the survivor deref; stderr:\n{stderr}"
    );
}

#[test]
fn c9bis_member_clone_binding_is_balanced() {
    let src = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = b
    b.next = a
    let keep: Rc<Node> = a
    let n: Rc<Node> = keep.next
    let v: i64 = n.val
    __aelys_collect()
    return v
}
"#;
    let Some((code, stderr)) = asan_run_cycles(src) else {
        return;
    };
    assert_eq!(
        code, 2,
        "the cloned read must yield keep.next.val=2; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "the 4th piece must keep the read balanced (no UAF); stderr:\n{stderr}"
    );
    let (a, f) = parse_stats(&stderr).unwrap_or_else(|| panic!("no stats:\n{stderr}"));
    assert_eq!(
        (a, f),
        (2, 2),
        "balanced clone read collects 2/2; stderr:\n{stderr}"
    );
}

#[test]
fn c9_multiple_reassign_releases_the_old_target() {
    let src = r#"
struct Node { next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { next: Rc::null() })
    let c: Rc<Node> = Rc::new(Node { next: Rc::null() })
    a.next = b
    a.next = c
    c.next = a
    __aelys_collect()
    return 0
}
"#;
    let Some((code, stderr)) = asan_run_cycles(src) else {
        return;
    };
    assert_eq!(code, 0, "multi-reassign run must exit 0; stderr:\n{stderr}");
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "multi-reassign lift must be ASan-clean; stderr:\n{stderr}"
    );
    let (a, f) = parse_stats(&stderr).unwrap_or_else(|| panic!("no stats:\n{stderr}"));
    assert_eq!(
        a, f,
        "every alloc must be freed (b acyclic + a/c cycle); stderr:\n{stderr}"
    );
}

#[test]
fn c9_self_cycle_is_balanced() {
    let src = r#"
struct Node { next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { next: Rc::null() })
    a.next = a
    __aelys_collect()
    return 0
}
"#;
    let Some((code, stderr)) = asan_run_cycles(src) else {
        return;
    };
    assert_eq!(code, 0, "self-cycle run must exit 0; stderr:\n{stderr}");
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "self-cycle lift must be ASan-clean (no double-free); stderr:\n{stderr}"
    );
    let (a, f) = parse_stats(&stderr).unwrap_or_else(|| panic!("no stats:\n{stderr}"));
    assert_eq!(
        (a, f),
        (1, 1),
        "self-cycle must be collected (1/1); stderr:\n{stderr}"
    );
}

#[test]
fn c9_reassign_to_null_releases_old() {
    let src = r#"
struct Node { next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { next: Rc::null() })
    a.next = b
    a.next = Rc::null()
    __aelys_collect()
    return 0
}
"#;
    let Some((code, stderr)) = asan_run_cycles(src) else {
        return;
    };
    assert_eq!(
        code, 0,
        "reassign-to-null run must exit 0; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "reassign-to-null lift must be ASan-clean; stderr:\n{stderr}"
    );
    let (a, f) = parse_stats(&stderr).unwrap_or_else(|| panic!("no stats:\n{stderr}"));
    assert_eq!(
        (a, f),
        (2, 2),
        "both Nodes acyclic -> freed (2/2); stderr:\n{stderr}"
    );
}

#[test]
fn c9_reassign_in_loop_is_balanced() {
    let src = r#"
struct Node { next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { next: Rc::null() })
    let mut i: i64 = 0
    while i < 3 {
        let b: Rc<Node> = Rc::new(Node { next: Rc::null() })
        a.next = b
        i = i + 1
    }
    a.next = Rc::null()
    __aelys_collect()
    return 0
}
"#;
    let Some((code, stderr)) = asan_run_cycles(src) else {
        return;
    };
    assert_eq!(code, 0, "loop-reassign run must exit 0; stderr:\n{stderr}");
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "loop-reassign lift must be ASan-clean; stderr:\n{stderr}"
    );
    let (a, f) = parse_stats(&stderr).unwrap_or_else(|| panic!("no stats:\n{stderr}"));
    assert_eq!(
        a, f,
        "every loop allocation must be freed; stderr:\n{stderr}"
    );
}

#[test]
fn f1_field_reassign_fresh_rhs_is_balanced() {
    let src = r#"
struct Inner { v: i64 }
struct Holder { r: Rc<Inner> }
fn main() -> i64 {
    let a: Rc<Holder> = Rc::new(Holder { r: Rc::null() })
    a.r = Rc::new(Inner { v: 5 })
    a.r = Rc::null()
    __aelys_collect()
    return 0
}
"#;
    for variant in [RuntimeVariant::Rc, RuntimeVariant::RcCycles] {
        let Some((code, stderr)) = run_with_stats(src, variant) else {
            return;
        };
        assert_eq!(
            code, 0,
            "{variant:?}: fresh-rhs reassign must exit 0; stderr:\n{stderr}"
        );
        let (a, f) =
            parse_stats(&stderr).unwrap_or_else(|| panic!("{variant:?}: no stats:\n{stderr}"));
        assert_eq!(
            (a, f),
            (2, 2),
            "{variant:?}: a FRESH `Rc::new` field-rhs must NOT be over-retained \
             (allocs==frees once released); got {a}/{f}; stderr:\n{stderr}"
        );
    }
    let Some((code, stderr)) = asan_run_cycles(src) else {
        return;
    };
    assert_eq!(code, 0, "F1 ASan run must exit 0; stderr:\n{stderr}");
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "F1: the fresh field-rhs must be leak-free under LSan; stderr:\n{stderr}"
    );
}

#[test]
fn f2_garbage_cycle_holding_live_survivor_is_balanced() {
    let src = r#"
struct Node { val: i64, next: Rc<Node>, link: Rc<Node> }
fn main() -> i64 {
    let keep: Rc<Node> = Rc::new(Node { val: 99, next: Rc::null(), link: Rc::null() })
    {
        let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null(), link: Rc::null() })
        let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null(), link: Rc::null() })
        a.next = b
        b.next = a
        a.link = keep
    }
    __aelys_collect()
    let v: i64 = keep.val
    return v
}
"#;
    let Some((code, stderr)) = run_with_stats(src, RuntimeVariant::RcCycles) else {
        return;
    };
    assert_eq!(
        code, 99,
        "anti-UAF: keep.val must read 99 after collect (keep is live, never freed); \
         stderr:\n{stderr}"
    );
    let (a, f) = parse_stats(&stderr).unwrap_or_else(|| panic!("no stats:\n{stderr}"));
    assert_eq!(
        (a, f),
        (3, 3),
        "a garbage cycle holding a  live object must release it transitively \
         (allocs==frees no phantom+1; got {a}/{f}; stderr:\n{stderr}"
    );
    let Some((code, stderr)) = asan_run_cycles(src) else {
        return;
    };
    assert_eq!(code, 99, "F2 ASan run must exit 99; stderr:\n{stderr}");
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "F2: transitive release of the survivor must be leak-/UAF-clean; stderr:\n{stderr}"
    );
}

#[test]
fn c4_acyclic_program_idempotent_under_rc_cycles() {
    let src = r#"
fn touch(a: Rc<i64>) -> i64 { return 1 }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let b: Rc<i64> = a
    let _t: i64 = touch(b)
    return Rc::get(a)
}
"#;
    let Some((code_cyc, stderr_cyc)) = run_with_stats(src, RuntimeVariant::RcCycles) else {
        return;
    };
    let Some((code_rc, _)) = run_with_stats(src, RuntimeVariant::Rc) else {
        return;
    };
    assert_eq!(
        code_cyc, code_rc,
        "rc+cycles must match rc exit on an acyclic program"
    );
    assert_eq!(
        code_cyc, 7,
        "expected the value read through the handle; stderr:\n{stderr_cyc}"
    );
    let (a, f) = parse_stats(&stderr_cyc).unwrap_or_else(|| panic!("no stats:\n{stderr_cyc}"));
    assert_eq!(
        (a, f),
        (1, 1),
        "shared acyclic Rc frees once under rc+cycles; stderr:\n{stderr_cyc}"
    );
}

#[test]
fn c5_rc_null_allocates_nothing() {
    let src = r#"
struct Node { next: Rc<Node> }
fn use_n(n: Rc<Node>) -> i64 { return 0 }
fn main() -> i64 {
    let z: Rc<Node> = Rc::null()
    return use_n(z)
}
"#;
    let Some((code, stderr)) = run_with_stats(src, RuntimeVariant::RcCycles) else {
        return;
    };
    assert_eq!(code, 0, "Rc::null program must exit 0; stderr:\n{stderr}");
    let (a, f) = parse_stats(&stderr).unwrap_or_else(|| panic!("no stats:\n{stderr}"));
    assert_eq!(
        (a, f),
        (0, 0),
        "Rc::null must allocate/free nothing; stderr:\n{stderr}"
    );
}

#[test]
fn c5_rc_null_emits_no_alloc_in_ir() {
    let ir = rc_ir(
        r#"
struct Node { next: Rc<Node> }
fn use_n(n: Rc<Node>) -> i64 { return 0 }
fn mk() -> i64 {
    let a: Rc<Node> = Rc::new(Node { next: Rc::null() })
    return use_n(a)
}
fn main() -> i64 { return mk() }
"#,
    );
    let header_inits = ir.matches("store i32 1, ptr %rc_alloc_raw").count();
    assert_eq!(
        header_inits, 1,
        "exactly one Rc header init (Rc::new); Rc::null must not allocate. IR:\n{ir}"
    );
}

#[test]
fn c6_field_reassign_emits_the_balanced_lift() {
    let ir = rc_ir(
        r#"
struct Node { next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { next: Rc::null() })
    a.next = b
    return 0
}
"#,
    );
    assert!(
        ir.contains("__aelys_rc_release"),
        "the lift must release the old field value; IR:\n{ir}"
    );
    assert!(
        ir.contains("__aelys_rc_retain"),
        "the lift must retain the new field value; IR:\n{ir}"
    );
}

#[test]
fn c6_nominal_carrier_field_reassign_through_handle_is_rejected() {
    let src = r#"
struct Inner { r: Rc<i64> }
struct Outer { b: Inner }
fn main() -> i64 {
    let o: Rc<Outer> = Rc::new(Outer { b: Inner { r: Rc::new(1) } })
    let other: Inner = Inner { r: Rc::new(2) }
    o.b = other
    return 0
}
"#;
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    let res = compile_file_with_llvm_variant(
        &source_path,
        OptimizationLevel::None,
        true,
        RuntimeVariant::RcCycles,
    );
    assert!(
        res.is_err(),
        "reassigning a nominal-carrier field through an Rc handle must be rejected"
    );
}

#[test]
fn c8_default_runtime_is_still_rc() {
    assert_eq!(
        RuntimeVariant::default(),
        RuntimeVariant::Rc,
        "default must stay Rc"
    );
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(7)
    return Rc::get(r)
}
"#,
    )
    .expect("write source");
    match compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
        Ok(()) | Err(_) => {}
    }
}

#[test]
fn c8_runtime_variant_parse_and_suffix() {
    assert_eq!(
        RuntimeVariant::parse("rc+cycles"),
        Some(RuntimeVariant::RcCycles)
    );
    assert_eq!(RuntimeVariant::parse("rc"), Some(RuntimeVariant::Rc));
    assert_eq!(RuntimeVariant::parse("leak"), Some(RuntimeVariant::Leak));
    assert_eq!(RuntimeVariant::parse("nonsense"), None);
    assert_eq!(RuntimeVariant::RcCycles.lib_suffix(), "rc-cycles");
}

#[test]
fn c8_cycles_archive_exports_collector_and_common_exports_collect() {
    let Some(archive) = find_core_archive("libaelys-core-rc-cycles.a") else {
        panic!("expected libaelys-core-rc-cycles.a (build the workspace first)");
    };
    let Ok(out) = Command::new("nm").arg(&archive).output() else {
        eprintln!("nm unavailable; skipping symbol assertion");
        return;
    };
    let syms = String::from_utf8_lossy(&out.stdout);
    for sym in [
        "__aelys_cycle_collect",
        "__aelys_collect",
        "__aelys_rc_retain",
        "__aelys_rc_release",
        "__aelys_rc_type_table",
    ] {
        assert!(
            syms.contains(sym),
            "libaelys-core-rc-cycles.a must mention {sym}; nm:\n{syms}"
        );
    }
}

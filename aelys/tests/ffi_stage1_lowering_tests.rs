use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

// gates the call site on , so every call below carries its own block
const LABS: &str =
    "unsafe extern fn labs(x: i64) -> i64\n\nfn main() -> i64 {\n    unsafe { return labs(-37) }\n}\n";
const FFSL: &str =
    "unsafe extern fn ffsl(x: i64) -> i64\n\nfn main() -> i64 {\n    unsafe { return ffsl(1024) }\n}\n";

const LABS_BARE: &str =
    "unsafe extern fn labs(x: i64) -> i64\n\nfn main() -> i64 {\n    return labs(-37)\n}\n";
const FFSL_BARE: &str =
    "unsafe extern fn ffsl(x: i64) -> i64\n\nfn main() -> i64 {\n    return ffsl(1024)\n}\n";

#[test]
fn f3_1_bis_the_old_bare_spelling_of_the_libc_call_is_now_e0617() {
    for (level, opt) in LEVELS {
        for (id, source) in [("F3-1-bis", LABS_BARE), ("F3-3-bis", FFSL_BARE)] {
            let dir = tempdir().expect("tempdir");
            let root = dir.path().join("root.aelys");
            fs::write(&root, source).expect("write fixture");
            let rendered = match lower_file_to_air(&root, *opt) {
                Ok(_) => panic!("{id} at {level}: the bare call MUST be rejected\n{source}"),
                Err(rendered) => rendered,
            };
            assert!(
                rendered.contains("E0617"),
                "{id} at {level}: the rejection MUST be E0617\nrendered:\n{rendered}"
            );
        }
    }
}

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
}

struct Built {
    _dir: tempfile::TempDir,
    exe: PathBuf,
}

fn build(id: &str, level: &str, source: &str, opt: OptimizationLevel) -> Built {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, source).expect("write fixture");
    if let Err(err) = compile_file_with_llvm_variant(&root, opt, false, RuntimeVariant::Rc) {
        panic!("{id} at {level}: the program MUST compile and link\nerror:\n{err}");
    }
    let exe = exe_path_for(&root);
    Built { _dir: dir, exe }
}

fn exit_code(id: &str, level: &str, exe: &Path) -> i32 {
    let out = Command::new(exe).output().expect("run executable");
    out.status.code().unwrap_or_else(|| {
        panic!(
            "{id} at {level}: the program MUST exit normally\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        )
    })
}

fn nm(args: &[&str], exe: &Path) -> Option<String> {
    let out = Command::new("nm").args(args).arg(exe).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

fn defines_no_body_for(id: &str, level: &str, exe: &Path, symbol: &str) {
    let Some(table) = nm(&[], exe) else {
        eprintln!("{id} at {level}: nm unavailable, skipping the body check");
        return;
    };
    assert!(
        !table.contains(&format!(" T {symbol}")),
        "{id} at {level}: `{symbol}` MUST NOT have an aelys body in the image\nnm:\n{table}"
    );
}

#[test]
fn f3_1_the_libc_answer_comes_back_and_the_symbol_stays_undefined() {
    let built = build("F3-1", "-O0", LABS, OptimizationLevel::None);
    assert_eq!(
        exit_code("F3-1", "-O0", &built.exe),
        37,
        "F3-1: `labs(-37)` MUST answer 37"
    );
    defines_no_body_for("F3-1", "-O0", &built.exe, "labs");
    let Some(undefined) = nm(&["-u"], &built.exe) else {
        eprintln!("F3-1: nm unavailable, skipping the undefined-symbol check");
        return;
    };
    assert!(
        undefined.contains("U labs@GLIBC_"),
        "F3-1: `labs` MUST be an undefined symbol resolved by libc\nnm -u:\n{undefined}"
    );
}

#[test]
fn f3_2_the_libc_answer_is_the_same_at_every_level() {
    for (level, opt) in LEVELS {
        let built = build("F3-2", level, LABS, *opt);
        assert_eq!(
            exit_code("F3-2", level, &built.exe),
            37,
            "F3-2 at {level}: `labs(-37)` MUST answer 37"
        );
        defines_no_body_for("F3-2", level, &built.exe, "labs");
    }
}

#[test]
fn f3_3_a_second_libc_symbol_answers_and_survives_every_level() {
    for (level, opt) in LEVELS {
        let built = build("F3-3", level, FFSL, *opt);
        assert_eq!(
            exit_code("F3-3", level, &built.exe),
            11,
            "F3-3 at {level}: `ffsl(1024)` MUST answer 11"
        );
        defines_no_body_for("F3-3", level, &built.exe, "ffsl");
        // llvm rewrites `labs` into an intrinsic and leaves `ffsl` alone, so only this row keeps the undefined symbol
        let Some(undefined) = nm(&["-u"], &built.exe) else {
            eprintln!("F3-3 at {level}: nm unavailable, skipping the undefined-symbol check");
            continue;
        };
        assert!(
            undefined.contains("U ffsl@GLIBC_"),
            "F3-3 at {level}: `ffsl` MUST stay an undefined symbol\nnm -u:\n{undefined}"
        );
    }
}

#[test]
fn the_declaration_prints_as_an_extern_air_function() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, LABS).expect("write fixture");
    let air = lower_file_to_air(&root, OptimizationLevel::None)
        .expect("the declaration MUST lower without a diagnostic");
    let printed = aelys_air::print::print_program(&air);
    assert!(
        printed.contains("fn labs(x: i64) -> i64  [extern]"),
        "the declaration MUST print as an extern air function, found:\n{printed}"
    );
    let labs = air
        .functions
        .iter()
        .find(|f| f.name == "labs")
        .expect("the declaration MUST reach the air program");
    assert!(labs.is_extern, "the declaration MUST be extern");
    assert!(labs.blocks.is_empty(), "an extern declaration has no body");
    assert!(
        matches!(labs.calling_conv, aelys_air::CallingConv::C),
        "a foreign declaration carries the c convention"
    );
}

#[test]
fn the_declaration_emits_a_declare_and_never_a_define() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, LABS).expect("write fixture");
    compile_file_with_llvm_variant(&root, OptimizationLevel::None, true, RuntimeVariant::Rc)
        .expect("the declaration MUST reach llvm");
    let ir = fs::read_to_string(root.with_extension("ll")).expect("read emitted .ll");
    assert!(
        ir.contains("declare i64 @labs(i64"),
        "the declaration MUST emit a declare, found:\n{ir}"
    );
    for line in ir.lines() {
        assert!(
            !(line.starts_with("define") && line.contains("@labs(")),
            "the declaration MUST NOT emit a define, found:\n{line}"
        );
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

#[test]
fn the_return_type_normalisation_lives_at_one_site() {
    let text = fs::read_to_string(repo_root().join("air/src/lower/program.rs"))
        .expect("read the lowering source");
    assert_eq!(
        text.matches("if ret_ty == AirType::Ptr(Box::new(AirType::Void))")
            .count(),
        1,
        "a second copy of the return normalisation would drift without a witness"
    );
}

#[test]
fn f3_46_the_call_to_the_foreign_symbol_survives_every_level() {
    for (level, opt) in LEVELS {
        let dir = tempdir().expect("tempdir");
        let root = dir.path().join("root.aelys");
        fs::write(&root, FFSL).expect("write fixture");
        if let Err(err) = compile_file_with_llvm_variant(&root, *opt, true, RuntimeVariant::Rc) {
            panic!("F3-46 at {level}: the program MUST compile\nerror:\n{err}");
        }
        let ir = fs::read_to_string(root.with_extension("ll")).expect("read emitted .ll");
        assert!(
            ir.contains("declare i64 @ffsl(i64"),
            "F3-46 at {level}: the declaration MUST stay a declare\n{ir}"
        );
        assert!(
            ir.contains("call i64 @ffsl("),
            "F3-46 at {level}: the call MUST still be there; a body-less callee that the \
             optimiser felt free to expand would leave none\n{ir}"
        );
        let built = build("F3-46", level, FFSL, *opt);
        assert_eq!(
            exit_code("F3-46", level, &built.exe),
            11,
            "F3-46 at {level}: and it MUST still answer"
        );
        let Some(undefined) = nm(&["-u"], &built.exe) else {
            eprintln!("F3-46 at {level}: nm unavailable, skipping the undefined-symbol check");
            continue;
        };
        assert!(
            undefined.contains("U ffsl@GLIBC_"),
            "F3-46 at {level}: the symbol MUST reach the linker\nnm -u:\n{undefined}"
        );
    }
}

// the inliner elects every extern: body_size is zero, so analyze.rs answers inline, and the call is held only by the arity test below
#[test]
fn f3_47_the_only_thing_that_holds_the_inliner_off_an_extern_is_one_line() {
    let root = repo_root();
    let expand = fs::read_to_string(root.join("opt/src/passes/inline/expand.rs"))
        .expect("read the inline expansion source");
    assert_eq!(
        expand.matches("if body.len() != 1").count(),
        1,
        "F3-47: an extern has no body at all, so this is the test that answers None for it and \
         leaves the call standing; nobody chose it for that, and a second copy or a rewrite of it \
         would take the guard away without a diagnostic"
    );
    let analyze = fs::read_to_string(root.join("opt/src/passes/inline/analyze.rs"))
        .expect("read the inline analysis source");
    assert_eq!(
        analyze.matches("if info.body_size <= 3").count(),
        1,
        "F3-47: and this is what elects it: a declaration measures zero, which is trivial by this \
         rule"
    );
}

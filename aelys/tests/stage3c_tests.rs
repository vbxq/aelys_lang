use aelys_driver::{compile_file_with_llvm, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::process::{Command, Output};
use tempfile::tempdir;

mod common;
use common::{exe_path_for, linker_unavailable};

const LEVELS: &[OptimizationLevel] = &[
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const REJECT_LEVELS: &[OptimizationLevel] = &[OptimizationLevel::None, OptimizationLevel::Standard];

fn lower_err(src: &str, level: OptimizationLevel) -> Result<(), String> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    lower_file_to_air(&source_path, level)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn assert_rejects_with(src: &str, code: &str) {
    for level in REJECT_LEVELS {
        match lower_err(src, *level) {
            Ok(()) => panic!("expected reject with {code} at {level:?}, but it compiled"),
            Err(err) => assert!(
                err.contains(&format!("[{code}]")),
                "at {level:?} the reject must carry {code}: {err}"
            ),
        }
    }
}

fn run(src: &str, level: OptimizationLevel) -> Option<Output> {
    let _pin = common::pin_legs("run", 1);
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm(&source_path, level, false) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                common::require_linker_skip(
                    "a skipped value row carries no runtime evidence at all",
                );
                return None;
            }
            panic!("compilation/link should succeed at {level:?}: {err}");
        }
    }

    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        common::require_linker_skip("a skipped value row carries no runtime evidence at all");
        return None;
    }
    common::note_leg();
    Some(Command::new(&exe).output().expect("run compiled exe"))
}

#[cfg(unix)]
fn abort_signal(out: &Output) -> Option<i32> {
    use std::os::unix::process::ExitStatusExt;
    out.status.signal()
}

#[cfg(not(unix))]
fn abort_signal(_out: &Output) -> Option<i32> {
    None
}

// every -o level so the old 136-vs-232 / 139-vs-0 cross-o divergence cannot come back.
fn assert_traps_every_level(label: &str, src: &str) {
    let _pin = common::pin_legs(label, LEVELS.len());
    for level in LEVELS {
        let Some(out) = run(src, *level) else { return };
        let stderr = String::from_utf8_lossy(&out.stderr);
        if cfg!(unix) {
            assert_eq!(
                abort_signal(&out),
                Some(6),
                "[{label}] must trap by SIGABRT at {level:?}, got code {:?}; stderr:\n{stderr}",
                out.status.code()
            );
        } else {
            assert_eq!(
                out.status.code(),
                Some(134),
                "[{label}] must trap (134) at {level:?}; stderr:\n{stderr}"
            );
        }
    }
}

fn assert_returns_every_level(label: &str, src: &str, expected: i32) {
    let _pin = common::pin_legs(label, LEVELS.len());
    for level in LEVELS {
        let Some(out) = run(src, *level) else { return };
        assert_eq!(
            out.status.code(),
            Some(expected),
            "[{label}] must return {expected} at {level:?}; got {:?}",
            out.status.code()
        );
    }
}

const INT_MIN_DIV: &str = r#"
use std::process::ExitCode; fn main() -> ExitCode {
    let a = 0 - 9223372036854775807 - 1;
    let b = 0 - 1;
    return ExitCode::from((a / b) as u8);
}
"#;

const INT_MIN_REM: &str = r#"
use std::process::ExitCode; fn main() -> ExitCode {
    let a = 0 - 9223372036854775807 - 1;
    let b = 0 - 1;
    return ExitCode::from((a % b) as u8);
}
"#;

#[test]
fn defect1_int_min_div_traps_every_level() {
    assert_traps_every_level("INT_MIN / -1", INT_MIN_DIV);
}

#[test]
fn defect1_int_min_rem_traps_every_level() {
    assert_traps_every_level("INT_MIN % -1", INT_MIN_REM);
}

#[test]
fn defect1_ordinary_division_still_correct() {
    assert_returns_every_level("7 / 2", "use std::process::ExitCode; fn main() -> ExitCode { return ExitCode::from((7 / 2) as u8); }\n", 3);
    assert_returns_every_level(
        "-8 / -1",
        "use std::process::ExitCode; fn main() -> ExitCode { let a = 0 - 8; let b = 0 - 1; return ExitCode::from((a / b) as u8); }\n",
        8,
    );
    assert_returns_every_level(
        "INT_MIN / 2",
        "use std::process::ExitCode; fn main() -> ExitCode { let a = 0 - 9223372036854775807 - 1; return ExitCode::from((a / 2 + 4611686018427387904) as u8); }\n",
        0,
    );
    assert_returns_every_level("7 % 3", "use std::process::ExitCode; fn main() -> ExitCode { return ExitCode::from((7 % 3) as u8); }\n", 1);
}

const MUT_REF_IMMUTABLE: &str = r#"
use std::process::ExitCode; fn main() -> ExitCode {
    let x = 1;
    let a = &mut x;
    *a = 2;
    return ExitCode::from(x as u8);
}
"#;

#[test]
fn defect2_mut_ref_of_immutable_binding_is_e0417() {
    assert_rejects_with(MUT_REF_IMMUTABLE, "E0417");
}

#[test]
fn defect2_mut_ref_of_mutable_binding_compiles_and_mutates() {
    assert_returns_every_level(
        "&mut let-mut",
        "use std::process::ExitCode; fn main() -> ExitCode { let mut x = 1; let a = &mut x; *a = 2; return ExitCode::from(x as u8); }\n",
        2,
    );
}

#[test]
fn defect2_immutable_ref_of_immutable_binding_compiles() {
    assert_returns_every_level(
        "&x immutable",
        "use std::process::ExitCode; fn main() -> ExitCode { let x = 5; let a = &x; return ExitCode::from((*a) as u8); }\n",
        5,
    );
}

const RC_GET_NULL: &str = r#"
use std::process::ExitCode; fn main() -> ExitCode {
    let r: Rc<i64> = Rc::null();
    return ExitCode::from(Rc::get(r) as u8);
}
"#;

#[test]
fn defect3_rc_get_null_traps_every_level() {
    assert_traps_every_level("Rc::get(Rc::null())", RC_GET_NULL);
}

#[test]
fn defect3_rc_get_of_real_rc_returns_value() {
    assert_returns_every_level(
        "Rc::get(Rc::new(42))",
        "use std::process::ExitCode; fn main() -> ExitCode { let r = Rc::new(42); return ExitCode::from(Rc::get(r) as u8); }\n",
        42,
    );
}

const NESTED_SHADOW: &str = r#"
use std::process::ExitCode; fn pick<A, B>(a: A, b: B) -> A {
    a
}
fn other() -> i64 {
    fn pick<A, B>(a: A, b: B) -> B {
        b
    }
    pick(1, 2)
}
fn main() -> ExitCode {
    return ExitCode::from(pick(7, 8) as u8);
}
"#;

#[test]
fn defect4_nested_generic_shadow_is_e0418() {
    assert_rejects_with(NESTED_SHADOW, "E0418");
}

#[test]
fn defect4_unique_nested_name_compiles() {
    assert_returns_every_level(
        "unique nested name",
        r#"
use std::process::ExitCode; fn pick<A, B>(a: A, b: B) -> A { a }
fn other() -> i64 {
    fn choose<A, B>(a: A, b: B) -> B { b }
    choose(1, 2)
}
fn main() -> ExitCode { return ExitCode::from(pick(7, 8) as u8); }
"#,
        7,
    );
}

#[test]
fn defect4_plain_top_level_generic_call_works() {
    assert_returns_every_level(
        "plain top-level",
        r#"
use std::process::ExitCode; fn pick<A, B>(a: A, b: B) -> A { a }
fn main() -> ExitCode { return ExitCode::from(pick(7, 8) as u8); }
"#,
        7,
    );
}

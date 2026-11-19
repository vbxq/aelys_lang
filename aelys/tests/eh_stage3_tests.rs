// 3a .unwrap()/.expect(lit) · 3b .map_error · 3c catch · 3d never restriction, .into_ok(), unsafe, .unwrap_unchecked()

use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::fs;
use std::process::{Command, Output};
use tempfile::tempdir;

mod common;
use common::{exe_path_for, linker_unavailable};

fn reject(src: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect_err("compilation should be rejected")
        .to_string()
}

fn run(src: &str) -> Option<Output> {
    let _pin = common::pin_legs("run", 1);
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    match compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                common::require_linker_skip(
                    "a skipped value row carries no runtime evidence at all",
                );
                return None;
            }
            panic!("compilation/link should succeed: {err}");
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

#[test]
fn unwrap_ok_returns_value() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn ok_val() -> Result<i64, E> { Result::Ok(9) }

fn main() -> ExitCode { return ExitCode::from(ok_val().unwrap() as u8); }
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        9,
        "unwrap on Ok(9) should return 9"
    );
}

#[test]
fn unwrap_err_aborts_with_default_message() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn err_val() -> Result<i64, E> { Result::Err(E::X) }

fn main() -> ExitCode { return ExitCode::from(err_val().unwrap() as u8); }
"#;
    let Some(out) = run(src) else { return };
    assert!(
        !out.status.success(),
        "unwrap on Err should abort (non-zero exit)"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("called .unwrap() on an Err value"),
        "stderr should carry the default unwrap message, got: {stderr}"
    );
}

#[test]
fn expect_err_aborts_with_literal_message() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn err_val() -> Result<i64, E> { Result::Err(E::X) }

fn main() -> ExitCode { return ExitCode::from(err_val().expect("boom") as u8); }
"#;
    let Some(out) = run(src) else { return };
    assert!(
        !out.status.success(),
        "expect on Err should abort (non-zero exit)"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("boom"),
        "stderr should carry the expect message 'boom', got: {stderr}"
    );
}

#[test]
fn expect_ok_returns_value() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn ok_val() -> Result<i64, E> { Result::Ok(7) }

fn main() -> ExitCode { return ExitCode::from(ok_val().expect("unreachable") as u8); }
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        7,
        "expect on Ok(7) should return 7"
    );
}

#[test]
fn expect_non_literal_is_rejected() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn ok_val() -> Result<i64, E> { Result::Ok(1) }

fn main() -> ExitCode {
    let msg: i64 = 7;
    return ExitCode::from(ok_val().expect(msg) as u8);
}
"#;
    let err = reject(src);
    assert!(
        err.contains("[result]") && err.contains("string literal"),
        "expected a [result] string-literal rejection, got: {err}"
    );
}

#[test]
fn struct_field_named_unwrap_still_compiles_and_runs() {
    let src = r#"
use std::process::ExitCode; struct S { unwrap: fn() -> i64 }

fn five() -> i64 { 5 }

fn main() -> ExitCode {
    let s = S { unwrap: five };
    return ExitCode::from(s.unwrap() as u8);
}
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        5,
        "calling the struct field `unwrap` should return 5"
    );
}

#[test]
fn unwrap_on_non_result_still_errors_without_swallow() {
    let src = r#"
use std::process::ExitCode; fn main() -> ExitCode {
    let x: i64 = 42;
    return ExitCode::from(x.unwrap() as u8);
}
"#;
    let err = reject(src);
    assert!(
        !err.contains("[result]"),
        "i64 `.unwrap()` must not be swallowed by the seam, got: {err}"
    );
    assert!(
        !err.is_empty(),
        "i64 `.unwrap()` should still be a compile error"
    );
}

#[test]
fn map_error_transforms_error_payload() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum EA { Lo, Hi }
enum EB { Mild, Worse }

fn widen(e: EA) -> EB {
    match e {
        EA::Lo => return EB::Mild,
        EA::Hi => return EB::Worse,
    }
}

fn get() -> Result<i64, EA> { Result::Err(EA::Hi) }

fn main() -> ExitCode {
    let r: Result<i64, EB> = get().map_error(widen);
    match r {
        Result::Ok(v)  => return ExitCode::from(v as u8),
        Result::Err(e) => match e {
            EB::Mild  => return ExitCode::from(1),
            EB::Worse => return ExitCode::from(2),
        },
    }
}
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        2,
        "EA::Hi must flow through widen to EB::Worse (exit 2), proving f transformed the Err payload"
    );
}

#[test]
fn map_error_ok_passes_through() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum EA { Bad }
enum EB { Worse }

fn widen(e: EA) -> EB { EB::Worse }

fn get_ok() -> Result<i64, EA> { Result::Ok(42) }

fn main() -> ExitCode {
    let r: Result<i64, EB> = get_ok().map_error(widen);
    match r {
        Result::Ok(v)  => return ExitCode::from(v as u8),
        Result::Err(e) => return ExitCode::from(99),
    }
}
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        42,
        "Ok(42) must pass through map_error unchanged (exit 42), f is never applied on the Ok path"
    );
}

#[test]
fn map_error_bare_statement_is_must_use_flagged() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum EA { Bad }
enum EB { Worse }

fn widen(e: EA) -> EB { EB::Worse }

fn get() -> Result<i64, EA> { Result::Ok(1) }

fn main() {
    get().map_error(widen);
}
"#;
    let err = reject(src);
    assert!(
        err.contains("[must-use]"),
        "a dropped `x.map_error(f)` still yields a Result and must be must-use flagged, got: {err}"
    );
}

#[test]
fn map_error_discard_twin_compiles_and_runs() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum EA { Bad }
enum EB { Worse }

fn widen(e: EA) -> EB { EB::Worse }

fn get() -> Result<i64, EA> { Result::Ok(1) }

fn main() {
    let _ = get().map_error(widen);
}
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        0,
        "discard get().map_error(widen); return 0 must compile and exit 0"
    );
}

#[test]
fn map_error_evaluates_f_exactly_once() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum EA { Bad }
enum EB { Worse }

fn tap(e: EA) -> EB {
    print!("F");
    EB::Worse
}

fn get_err() -> Result<i64, EA> { Result::Err(EA::Bad) }

fn main() -> ExitCode {
    let r: Result<i64, EB> = get_err().map_error(tap);
    match r {
        Result::Ok(v)  => return ExitCode::from(v as u8),
        Result::Err(e) => return ExitCode::SUCCESS,
    }
}
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        0,
        "the Err path must run to exit 0"
    );
    let hits = out.stdout.iter().filter(|&&b| b == b'F').count();
    assert_eq!(
        hits,
        1,
        "f must be evaluated exactly once (one 'F'), not zero and not twice; stdout: {:?}",
        String::from_utf8_lossy(&out.stdout)
    );
}

#[test]
fn catch_arms_discriminate_each_variant() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { A, B }

fn get_a() -> Result<i64, E> { Result::Err(E::A) }
fn get_b() -> Result<i64, E> { Result::Err(E::B) }

fn main() -> ExitCode {
    let a: i64 = match get_a() { Ok(v) => v,
        Err(E::A) => 1,
        Err(E::B) => 2,
    };
    let b: i64 = match get_b() { Ok(v) => v,
        Err(E::A) => 1,
        Err(E::B) => 2,
    };
    return ExitCode::from((a * 10 + b) as u8);
}
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        12,
        "E::A must select arm 1 and E::B arm 2 (10*1 + 2 = 12), proving the right arm ran per variant"
    );
}

// 3c: a non-exhaustive discriminating catch is rejected by the reused match exhaustiveness check
#[test]
fn catch_arms_non_exhaustive_is_rejected() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { A, B }

fn get() -> Result<i64, E> { Result::Err(E::B) }

fn main() -> ExitCode {
    return ExitCode::from((match get() { Ok(v) => v,
        Err(E::A) => 1,
    }) as u8);
}
"#;
    let err = reject(src);
    assert!(
        err.contains("non-exhaustive"),
        "a catch missing the E::B arm must be rejected as non-exhaustive, got: {err}"
    );
}

#[test]
fn catch_yields_t_satisfies_must_use() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn get() -> Result<i64, E> { Result::Ok(1) }

fn main() -> ExitCode {
    match get() { Ok(v) => v, Err(e) => 0 };
    return ExitCode::from(5);
}
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        5,
        "a bare `get() catch |e| 0` statement compiles and the function returns 5"
    );
}

#[test]
fn never_rejected_outside_result_error_slot() {
    let cases: &[(&str, &str)] = &[
        (
            "fn return",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
fn f() -> Never { 5 }
fn main() { }
"#,
        ),
        (
            "fn param",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
fn g(x: Never) -> i64 { 1 }
fn main() { }
"#,
        ),
        (
            "struct field",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
struct S { f: Never }
fn main() { }
"#,
        ),
        (
            "let annotation",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
fn main() {
    let x: Never = 5;
}
"#,
        ),
        (
            "Result value slot",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
fn f() -> Result<Never, i64> { Result::Err(3) }
fn main() { }
"#,
        ),
        (
            "Option type arg",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
enum Option<T> { Some(T), None }
fn f() -> Option<Never> { Option::None }
fn main() { }
"#,
        ),
        (
            "Vec element",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
fn f() -> Vec<Never> { [] }
fn main() { }
"#,
        ),
    ];
    for (label, src) in cases {
        let err = reject(src);
        assert!(
            err.contains("[result]") && err.contains("Never"),
            "{label}: expected a [result] Never-restriction error, got: {err}"
        );
    }
}

#[test]
fn into_ok_runs_on_result_never() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }

fn parse_fixed() -> Result<i64, Never> { Result::Ok(42) }

fn main() -> ExitCode { return ExitCode::from(parse_fixed().into_ok() as u8); }
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        42,
        "into_ok on Result<i64, Never> should read the Ok payload (42) and never panic"
    );
}

#[test]
fn into_ok_on_fallible_result_is_rejected() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum RealErr { Boom }

fn maybe() -> Result<i64, RealErr> { Result::Ok(5) }

fn main() -> ExitCode { return ExitCode::from(maybe().into_ok() as u8); }
"#;
    let err = reject(src);
    assert!(
        err.contains("[result]") && err.contains("into_ok"),
        "into_ok on a fallible Result must be rejected, got: {err}"
    );
}

#[test]
fn unwrap_unchecked_in_unsafe_runs() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn mk_ok() -> Result<i64, E> { Result::Ok(5) }

fn main() -> ExitCode { return ExitCode::from((unsafe { mk_ok().unwrap_unchecked() }) as u8); }
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        5,
        "unwrap_unchecked inside unsafe on Ok(5) should return 5"
    );
}

#[test]
fn unwrap_unchecked_outside_unsafe_is_rejected() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn mk_ok() -> Result<i64, E> { Result::Ok(5) }

fn main() -> ExitCode { return ExitCode::from(mk_ok().unwrap_unchecked() as u8); }
"#;
    let err = reject(src);
    assert!(
        err.contains("[result]") && err.contains("unsafe"),
        "unwrap_unchecked outside unsafe must be rejected, got: {err}"
    );
}

#[test]
fn unsafe_block_is_transparent() {
    let src = r#"
use std::process::ExitCode; fn main() -> ExitCode { return ExitCode::from((unsafe { 6 + 6 }) as u8); }
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        12,
        "unsafe {{ 6 + 6 }} should evaluate to 12"
    );
}

#[test]
fn nested_unsafe_still_compiles_and_runs() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn mk() -> Result<i64, E> { Result::Ok(9) }

fn main() -> ExitCode { return ExitCode::from((unsafe { unsafe { mk().unwrap_unchecked() } }) as u8); }
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        9,
        "nested unsafe blocks should still compile and run to 9"
    );
}

// 3d: once the unsafe block closes the depth returns to zero (medium-5), so a later unwrap_unchecked is rejected
#[test]
fn unsafe_depth_not_leaked_after_block() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn mk() -> Result<i64, E> { Result::Ok(1) }

fn main() -> ExitCode {
    let a: i64 = unsafe { mk().unwrap_unchecked() };
    return ExitCode::from(mk().unwrap_unchecked() as u8);
}
"#;
    let err = reject(src);
    assert!(
        err.contains("[result]") && err.contains("unsafe"),
        "unwrap_unchecked after the unsafe block closes must be rejected (depth not leaked), got: {err}"
    );
}

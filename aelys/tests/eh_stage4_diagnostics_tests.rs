use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::fs;
use tempfile::tempdir;

fn reject(src: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect_err("compilation should be rejected")
        .to_string()
}

// (c): a cross-error-type `?` note points the user at .map_error
#[test]
fn question_mismatch_note_points_at_map_error() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum Fault { Bad }

fn inner() -> Result<i64, Fault> { Result::Err(Fault::Bad) }

fn outer() -> Result<i64, i64> {
    let v: i64 = inner()?;
    Result::Ok(v)
}

fn main() { }
"#;
    let err = reject(src);
    assert!(
        err.contains("[try]"),
        "keeps the [try] marker, got: {err}"
    );
    assert!(
        err.contains(".map_error"),
        "the mismatch note must point at .map_error, got: {err}"
    );
}

// (e) twin: a real non-exhaustive match still says match, proving the catch flag does not leak
#[test]
fn non_exhaustive_match_note_still_says_match() {
    let src = r#"
use std::process::ExitCode; enum E { A, B }

fn main() -> ExitCode {
    let e: E = E::A;
    return ExitCode::from((match e {
        E::A => 1,
    }) as u8);
}
"#;
    let err = reject(src);
    assert!(
        err.contains("non-exhaustive match"),
        "a real match must still render as a non-exhaustive match, got: {err}"
    );
    assert!(
        !err.contains("non-exhaustive catch"),
        "a real match must not be mislabeled catch, got: {err}"
    );
}

// (b): `?` on a result inside a function that does not return result is rejected
#[test]
fn question_outside_result_or_option_fn_is_rejected() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn get() -> Result<i64, E> { Result::Ok(1) }

fn main() {
    let v: i64 = get()?;
}
"#;
    let err = reject(src);
    assert!(
        err.contains("[try]"),
        "keeps the [try] marker, got: {err}"
    );
    assert!(
        err.contains("requires the function to return"),
        "using `?` outside a Result fn must explain the return-type requirement, got: {err}"
    );
}

// (b): `?` on a non-result/option operand is rejected
#[test]
fn question_operand_not_result_or_option_is_rejected() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn f() -> Result<i64, E> {
    let x: i64 = 3;
    let y: i64 = x?;
    Result::Ok(0)
}

fn main() { }
"#;
    let err = reject(src);
    assert!(
        err.contains("[try]"),
        "keeps the [try] marker, got: {err}"
    );
    assert!(
        err.contains("expects a `Result"),
        "`?` on an i64 must say it expects a Result/Option, got: {err}"
    );
}

// (d): into_ok on a result that can fail is rejected and names the offending error type
#[test]
fn into_ok_on_fallible_result_names_error_type() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum RealErr { Boom }

fn maybe() -> Result<i64, RealErr> { Result::Ok(5) }

fn main() -> ExitCode { return ExitCode::from(maybe().into_ok() as u8); }
"#;
    let err = reject(src);
    assert!(
        err.contains("[result]"),
        "keeps the [result] marker, got: {err}"
    );
    assert!(err.contains("into_ok"), "must name into_ok, got: {err}");
    assert!(
        err.contains("RealErr"),
        "must name the concrete error type that makes it fallible, got: {err}"
    );
}

// 3d: unwrap_unchecked outside an unsafe block is rejected and names unsafe
#[test]
fn unwrap_unchecked_outside_unsafe_names_unsafe() {
    let src = r#"
use std::process::ExitCode; enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn mk_ok() -> Result<i64, E> { Result::Ok(5) }

fn main() -> ExitCode { return ExitCode::from(mk_ok().unwrap_unchecked() as u8); }
"#;
    let err = reject(src);
    assert!(
        err.contains("[result]"),
        "keeps the [result] marker, got: {err}"
    );
    assert!(
        err.contains("unsafe"),
        "must name the unsafe requirement, got: {err}"
    );
}

// 3a: a non-literal .expect(msg) is rejected and says string literal
#[test]
fn expect_non_literal_names_string_literal() {
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
        err.contains("[result]"),
        "keeps the [result] marker, got: {err}"
    );
    assert!(
        err.contains("string literal"),
        "must say .expect takes a string literal, got: {err}"
    );
}

// 3d: never outside the error slot of a result is rejected and names never
#[test]
fn never_outside_result_error_slot_is_rejected() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }

fn f() -> Never { 5 }

fn main() { }
"#;
    let err = reject(src);
    assert!(
        err.contains("[result]"),
        "keeps the [result] marker, got: {err}"
    );
    assert!(
        err.contains("Never"),
        "the Never restriction must name Never, got: {err}"
    );
}

// 3b: map_error with a non-function argument is rejected and names map_error
#[test]
fn map_error_non_function_is_rejected() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum EA { Bad }

fn get() -> Result<i64, EA> { Result::Ok(1) }

fn main() {
    let x: i64 = 3;
    let r: Result<i64, EA> = get().map_error(x);
}
"#;
    let err = reject(src);
    assert!(
        err.contains("[result]"),
        "keeps the [result] marker, got: {err}"
    );
    assert!(
        err.contains("map_error"),
        "must name map_error and describe the expected fn shape, got: {err}"
    );
}

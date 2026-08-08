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

#[test]
fn must_use_menu_names_error_type_and_all_routes() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum MyErr { Boom }

fn get() -> Result<i64, MyErr> { return Result::Ok(1) }

fn main() -> i64 {
    get()
    return 0
}
"#;
    let err = reject(src);
    assert!(
        err.contains("[must-use]"),
        "keeps the [must-use] marker, got: {err}"
    );
    assert!(
        err.contains("MyErr"),
        "the menu must name the concrete error type MyErr, got: {err}"
    );
    assert!(
        err.contains("catch"),
        "the menu must list the catch route, got: {err}"
    );
    assert!(
        err.contains(".expect"),
        "the menu must list the .expect assert route, got: {err}"
    );
    assert!(
        err.contains("discard"),
        "the menu must list the discard route, got: {err}"
    );
}

// (c): a cross-error-type `?` note points the user at .map_error
#[test]
fn question_mismatch_note_points_at_map_error() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum Fault { Bad }

fn inner() -> Result<i64, Fault> { return Result::Err(Fault::Bad) }

fn outer() -> Result<i64, i64> {
    let v: i64 = inner()?
    return Result::Ok(v)
}

fn main() -> i64 { return 0 }
"#;
    let err = reject(src);
    assert!(
        err.contains("[?-stage1]"),
        "keeps the [?-stage1] marker, got: {err}"
    );
    assert!(
        err.contains(".map_error"),
        "the mismatch note must point at .map_error, got: {err}"
    );
}

#[test]
fn non_exhaustive_catch_note_says_catch() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { A, B }

fn get() -> Result<i64, E> { return Result::Err(E::B) }

fn main() -> i64 {
    return get() catch {
        E::A => 1,
    }
}
"#;
    let err = reject(src);
    assert!(
        err.contains("non-exhaustive"),
        "keeps the non-exhaustive substring, got: {err}"
    );
    assert!(
        err.contains("catch"),
        "a catch source must render as a non-exhaustive catch, got: {err}"
    );
}

// (e) twin: a real non-exhaustive match still says match, proving the catch flag does not leak
#[test]
fn non_exhaustive_match_note_still_says_match() {
    let src = r#"
enum E { A, B }

fn main() -> i64 {
    let e: E = E::A
    return match e {
        E::A => 1,
    }
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

fn get() -> Result<i64, E> { return Result::Ok(1) }

fn main() -> i64 {
    let v: i64 = get()?
    return 0
}
"#;
    let err = reject(src);
    assert!(
        err.contains("[?-stage1]"),
        "keeps the [?-stage1] marker, got: {err}"
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
    let x: i64 = 3
    let y: i64 = x?
    return Result::Ok(0)
}

fn main() -> i64 { return 0 }
"#;
    let err = reject(src);
    assert!(
        err.contains("[?-stage1]"),
        "keeps the [?-stage1] marker, got: {err}"
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
enum Result<T, E> { Ok(T), Err(E) }
enum RealErr { Boom }

fn maybe() -> Result<i64, RealErr> { return Result::Ok(5) }

fn main() -> i64 { return maybe().into_ok() }
"#;
    let err = reject(src);
    assert!(
        err.contains("[eh-stage3]"),
        "keeps the [eh-stage3] marker, got: {err}"
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
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn mk_ok() -> Result<i64, E> { return Result::Ok(5) }

fn main() -> i64 { return mk_ok().unwrap_unchecked() }
"#;
    let err = reject(src);
    assert!(
        err.contains("[eh-stage3]"),
        "keeps the [eh-stage3] marker, got: {err}"
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
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn ok_val() -> Result<i64, E> { return Result::Ok(1) }

fn main() -> i64 {
    let msg: i64 = 7
    return ok_val().expect(msg)
}
"#;
    let err = reject(src);
    assert!(
        err.contains("[eh-stage3]"),
        "keeps the [eh-stage3] marker, got: {err}"
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

fn f() -> Never { return 5 }

fn main() -> i64 { return 0 }
"#;
    let err = reject(src);
    assert!(
        err.contains("[eh-stage3]"),
        "keeps the [eh-stage3] marker, got: {err}"
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

fn get() -> Result<i64, EA> { return Result::Ok(1) }

fn main() -> i64 {
    let x: i64 = 3
    let r: Result<i64, EA> = get().map_error(x)
    return 0
}
"#;
    let err = reject(src);
    assert!(
        err.contains("[eh-stage3]"),
        "keeps the [eh-stage3] marker, got: {err}"
    );
    assert!(
        err.contains("map_error"),
        "must name map_error and describe the expected fn shape, got: {err}"
    );
}

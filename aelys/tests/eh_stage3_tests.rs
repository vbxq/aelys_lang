// 3a .unwrap()/.expect(lit) · 3b .map_error · 3c catch · 3d never restriction, .into_ok(), unsafe, .unwrap_unchecked()

use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
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

fn reject(src: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect_err("compilation should be rejected")
        .to_string()
}

fn run(src: &str) -> Option<Output> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    match compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                eprintln!("linker unavailable; skipping run assertion");
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
    Some(Command::new(&exe).output().expect("run compiled exe"))
}

#[test]
fn unwrap_ok_returns_value() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn ok_val() -> Result<i64, E> { return Result::Ok(9) }

fn main() -> i64 { return ok_val().unwrap() }
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
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn err_val() -> Result<i64, E> { return Result::Err(E::X) }

fn main() -> i64 { return err_val().unwrap() }
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
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn err_val() -> Result<i64, E> { return Result::Err(E::X) }

fn main() -> i64 { return err_val().expect("boom") }
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
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn ok_val() -> Result<i64, E> { return Result::Ok(7) }

fn main() -> i64 { return ok_val().expect("unreachable") }
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
        err.contains("[eh-stage3]") && err.contains("string literal"),
        "expected a [eh-stage3] string-literal rejection, got: {err}"
    );
}

#[test]
fn struct_field_named_unwrap_still_compiles_and_runs() {
    let src = r#"
struct S { unwrap: fn() -> i64 }

fn five() -> i64 { return 5 }

fn main() -> i64 {
    let s = S { unwrap: five }
    return s.unwrap()
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
fn main() -> i64 {
    let x: i64 = 42
    return x.unwrap()
}
"#;
    let err = reject(src);
    assert!(
        !err.contains("[eh-stage3]"),
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
enum Result<T, E> { Ok(T), Err(E) }
enum EA { Lo, Hi }
enum EB { Mild, Worse }

fn widen(e: EA) -> EB {
    match e {
        EA::Lo => return EB::Mild,
        EA::Hi => return EB::Worse,
    }
}

fn get() -> Result<i64, EA> { return Result::Err(EA::Hi) }

fn main() -> i64 {
    let r: Result<i64, EB> = get().map_error(widen)
    match r {
        Result::Ok(v)  => return v,
        Result::Err(e) => match e {
            EB::Mild  => return 1,
            EB::Worse => return 2,
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
enum Result<T, E> { Ok(T), Err(E) }
enum EA { Bad }
enum EB { Worse }

fn widen(e: EA) -> EB { return EB::Worse }

fn get_ok() -> Result<i64, EA> { return Result::Ok(42) }

fn main() -> i64 {
    let r: Result<i64, EB> = get_ok().map_error(widen)
    match r {
        Result::Ok(v)  => return v,
        Result::Err(e) => return 99,
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

fn widen(e: EA) -> EB { return EB::Worse }

fn get() -> Result<i64, EA> { return Result::Ok(1) }

fn main() -> i64 {
    get().map_error(widen)
    return 0
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

fn widen(e: EA) -> EB { return EB::Worse }

fn get() -> Result<i64, EA> { return Result::Ok(1) }

fn main() -> i64 {
    discard get().map_error(widen)
    return 0
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
enum Result<T, E> { Ok(T), Err(E) }
enum EA { Bad }
enum EB { Worse }

fn tap(e: EA) -> EB {
    print("F")
    return EB::Worse
}

fn get_err() -> Result<i64, EA> { return Result::Err(EA::Bad) }

fn main() -> i64 {
    let r: Result<i64, EB> = get_err().map_error(tap)
    match r {
        Result::Ok(v)  => return v,
        Result::Err(e) => return 0,
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
fn catch_binder_recovers_on_err_and_passes_ok_through() {
    let err_src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn get_err() -> Result<i64, E> { return Result::Err(E::X) }

fn main() -> i64 { return get_err() catch |e| 100 }
"#;
    if let Some(out) = run(err_src) {
        assert_eq!(
            out.status.code().unwrap_or(-1),
            100,
            "catch |e| on an Err should yield the fallback 100"
        );
    }

    let ok_src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn get_ok() -> Result<i64, E> { return Result::Ok(7) }

fn main() -> i64 { return get_ok() catch |e| 100 }
"#;
    if let Some(out) = run(ok_src) {
        assert_eq!(
            out.status.code().unwrap_or(-1),
            7,
            "catch |e| on an Ok should yield the Ok payload 7, the handler must not run"
        );
    }
}

#[test]
fn catch_arms_discriminate_each_variant() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { A, B }

fn get_a() -> Result<i64, E> { return Result::Err(E::A) }
fn get_b() -> Result<i64, E> { return Result::Err(E::B) }

fn main() -> i64 {
    let a: i64 = get_a() catch {
        E::A => 1,
        E::B => 2,
    }
    let b: i64 = get_b() catch {
        E::A => 1,
        E::B => 2,
    }
    return a * 10 + b
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
        "a catch missing the E::B arm must be rejected as non-exhaustive, got: {err}"
    );
}

#[test]
fn catch_wildcard_and_binder_are_exhaustive() {
    let wildcard_src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { A, B }

fn get() -> Result<i64, E> { return Result::Err(E::B) }

fn main() -> i64 {
    return get() catch {
        E::A => 1,
        _ => 2,
    }
}
"#;
    if let Some(out) = run(wildcard_src) {
        assert_eq!(
            out.status.code().unwrap_or(-1),
            2,
            "the _ arm covers E::B, so the exhaustive catch yields 2"
        );
    }

    let binder_src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { A, B }

fn get() -> Result<i64, E> { return Result::Err(E::B) }

fn main() -> i64 { return get() catch |e| 42 }
"#;
    if let Some(out) = run(binder_src) {
        assert_eq!(
            out.status.code().unwrap_or(-1),
            42,
            "catch |e| is a catch-all, so it compiles and yields 42"
        );
    }
}

#[test]
fn catch_yields_t_satisfies_must_use() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn get() -> Result<i64, E> { return Result::Ok(1) }

fn main() -> i64 {
    get() catch |e| 0
    return 5
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
fn catch_binder_reads_the_err_payload() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { A, B }

fn get() -> Result<i64, E> { return Result::Err(E::B) }

fn main() -> i64 {
    return get() catch |e| match e {
        E::A => 10,
        E::B => 20,
    }
}
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        20,
        "the binder e must be the Err payload E::B, so the inner match yields 20"
    );
}

#[test]
fn never_rejected_outside_result_error_slot() {
    let cases: &[(&str, &str)] = &[
        (
            "fn return",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
fn f() -> Never { return 5 }
fn main() -> i64 { return 0 }
"#,
        ),
        (
            "fn param",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
fn g(x: Never) -> i64 { return 1 }
fn main() -> i64 { return 0 }
"#,
        ),
        (
            "struct field",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
struct S { f: Never }
fn main() -> i64 { return 0 }
"#,
        ),
        (
            "let annotation",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
fn main() -> i64 {
    let x: Never = 5
    return 0
}
"#,
        ),
        (
            "Result value slot",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
fn f() -> Result<Never, i64> { return Result::Err(3) }
fn main() -> i64 { return 0 }
"#,
        ),
        (
            "Option type arg",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
enum Option<T> { Some(T), None }
fn f() -> Option<Never> { return Option::None }
fn main() -> i64 { return 0 }
"#,
        ),
        (
            "Vec element",
            r#"
enum Result<T, E> { Ok(T), Err(E) }
fn f() -> Vec<Never> { return [] }
fn main() -> i64 { return 0 }
"#,
        ),
    ];
    for (label, src) in cases {
        let err = reject(src);
        assert!(
            err.contains("[eh-stage3]") && err.contains("Never"),
            "{label}: expected a [eh-stage3] Never-restriction error, got: {err}"
        );
    }
}

#[test]
fn into_ok_runs_on_result_never() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }

fn parse_fixed() -> Result<i64, Never> { return Result::Ok(42) }

fn main() -> i64 { return parse_fixed().into_ok() }
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
enum Result<T, E> { Ok(T), Err(E) }
enum RealErr { Boom }

fn maybe() -> Result<i64, RealErr> { return Result::Ok(5) }

fn main() -> i64 { return maybe().into_ok() }
"#;
    let err = reject(src);
    assert!(
        err.contains("[eh-stage3]") && err.contains("into_ok"),
        "into_ok on a fallible Result must be rejected, got: {err}"
    );
}

// 3d: unwrap_unchecked inside an unsafe block reads the ok payload
#[test]
fn unwrap_unchecked_in_unsafe_runs() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn mk_ok() -> Result<i64, E> { return Result::Ok(5) }

fn main() -> i64 { return unsafe { mk_ok().unwrap_unchecked() } }
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        5,
        "unwrap_unchecked inside unsafe on Ok(5) should return 5"
    );
}

// 3d: unwrap_unchecked outside an unsafe block is a compile error
#[test]
fn unwrap_unchecked_outside_unsafe_is_rejected() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn mk_ok() -> Result<i64, E> { return Result::Ok(5) }

fn main() -> i64 { return mk_ok().unwrap_unchecked() }
"#;
    let err = reject(src);
    assert!(
        err.contains("[eh-stage3]") && err.contains("unsafe"),
        "unwrap_unchecked outside unsafe must be rejected, got: {err}"
    );
}

// 3d: an unsafe block is transparent, it yields the value of its inner block
#[test]
fn unsafe_block_is_transparent() {
    let src = r#"
fn main() -> i64 { return unsafe { 6 + 6 } }
"#;
    let Some(out) = run(src) else { return };
    assert_eq!(
        out.status.code().unwrap_or(-1),
        12,
        "unsafe {{ 6 + 6 }} should evaluate to 12"
    );
}

// 3d: nesting unsafe keeps the counter positive throughout, so the inner unwrap_unchecked compiles
#[test]
fn nested_unsafe_still_compiles_and_runs() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn mk() -> Result<i64, E> { return Result::Ok(9) }

fn main() -> i64 { return unsafe { unsafe { mk().unwrap_unchecked() } } }
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
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn mk() -> Result<i64, E> { return Result::Ok(1) }

fn main() -> i64 {
    let a: i64 = unsafe { mk().unwrap_unchecked() }
    return mk().unwrap_unchecked()
}
"#;
    let err = reject(src);
    assert!(
        err.contains("[eh-stage3]") && err.contains("unsafe"),
        "unwrap_unchecked after the unsafe block closes must be rejected (depth not leaked), got: {err}"
    );
}

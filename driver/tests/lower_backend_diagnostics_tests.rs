// TODO: those tests *are* going to break because I'll change the error handling system at some point.

use aelys_driver::lower_file_to_air;
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn write_temp_source(prefix: &str, source: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("woof")
        .as_nanos();
    path.push(format!("aelys_driver_{prefix}_{stamp}.aelys"));
    fs::write(&path, source).expect("failed to write temp aelys source");
    path
}

fn assert_lowering_error(source: &str, expected_fragment: &str) {
    let path = write_temp_source("lower_diag", source);
    let result = std::panic::catch_unwind(|| lower_file_to_air(&path, OptimizationLevel::None));
    let _ = fs::remove_file(&path);

    assert!(
        result.is_ok(),
        "lower_file_to_air panicked instead of returning a diagnostic error"
    );
    let err = match result.expect("panic already checked") {
        Ok(_) => panic!("expected lowering to fail"),
        Err(err) => err,
    };
    assert!(
        err.contains(expected_fragment),
        "expected lowering error to contain `{expected_fragment}`, got:\n{err}"
    );
}

#[test]
fn non_constant_array_size_is_reported_without_panic() {
    let src = r#"
fn test(n: i64) -> i64 {
    let arr: [i64; 10] = [0; n];
    arr[0]
}
"#;
    assert_lowering_error(
        src,
        "unsupported non-constant array size: ArraySized requires a constant integer size expression",
    );
}

#[test]
fn returning_stack_array_is_reported_without_panic() {
    let src = r#"
fn f() -> [i64; 3] {
    let arr = [1, 2, 3];
    return arr;
}
"#;
    assert_lowering_error(
        src,
        "cannot return stack-allocated array from function: stack arrays are deallocated when the function returns",
    );
}

#[test]
fn oversized_stack_array_is_reported_without_panic() {
    let src = r#"
fn big() -> i64 {
    let arr = [0; 200000];
    arr[0]
}
"#;
    assert_lowering_error(src, "stack array too large: [i64; 200000]");
}

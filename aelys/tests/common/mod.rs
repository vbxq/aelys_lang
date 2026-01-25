//! Common test utilities for Aelys tests
#![allow(dead_code)]

use aelys::run;
use aelys_runtime::Value;

/// Run Aelys source code and return the result value.
/// Panics on compilation or runtime errors.
pub fn run_aelys(source: &str) -> Value {
    run(source, "<test>").expect("Aelys execution should succeed")
}

/// Run Aelys source code and expect it to succeed.
/// Returns the Value result.
pub fn run_aelys_ok(source: &str) -> Value {
    run(source, "<test>").expect("Expected success but got error")
}

/// Run Aelys source code and expect it to fail.
/// Returns the error message.
pub fn run_aelys_err(source: &str) -> String {
    match run(source, "<test>") {
        Ok(v) => panic!("Expected error but got success: {:?}", v),
        Err(e) => e.to_string(),
    }
}

/// Run Aelys source code and check if it returns the expected integer.
pub fn assert_aelys_int(source: &str, expected: i64) {
    let result = run_aelys(source);
    assert_eq!(
        result.as_int(),
        Some(expected),
        "Expected int {} but got {:?}",
        expected,
        result
    );
}

/// Run Aelys source code and check if it returns the expected boolean.
pub fn assert_aelys_bool(source: &str, expected: bool) {
    let result = run_aelys(source);
    assert_eq!(
        result.as_bool(),
        Some(expected),
        "Expected bool {} but got {:?}",
        expected,
        result
    );
}

/// Run Aelys source code and check if it returns null.
pub fn assert_aelys_null(source: &str) {
    let result = run_aelys(source);
    assert!(result.is_null(), "Expected null but got {:?}", result);
}

/// Run Aelys source code and check if it returns an error containing the given substring.
pub fn assert_aelys_error_contains(source: &str, expected_substring: &str) {
    let err = run_aelys_err(source);
    assert!(
        err.contains(expected_substring),
        "Expected error containing '{}' but got: {}",
        expected_substring,
        err
    );
}

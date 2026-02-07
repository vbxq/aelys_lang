//! Common test utilities for Aelys tests
#![allow(dead_code)]

use aelys::{new_vm, run_with_vm_and_opt};
use aelys_opt::OptimizationLevel;
use aelys_runtime::Value;

/// Run Aelys source code and return the result value.
/// Panics on compilation or runtime errors.
/// Supports module imports (needs std.X).
pub fn run_aelys(source: &str) -> Value {
    let mut vm = new_vm().expect("Failed to create VM");
    run_with_vm_and_opt(&mut vm, source, "<test>", OptimizationLevel::Standard)
        .expect("Aelys execution should succeed")
}

/// Run Aelys source code and expect it to succeed.
/// Returns the Value result.
pub fn run_aelys_ok(source: &str) -> Value {
    let mut vm = new_vm().expect("Failed to create VM");
    run_with_vm_and_opt(&mut vm, source, "<test>", OptimizationLevel::Standard)
        .expect("Expected success but got error")
}

/// Run Aelys source code and expect it to fail.
/// Returns the error message.
pub fn run_aelys_err(source: &str) -> String {
    let mut vm = new_vm().expect("Failed to create VM");
    match run_with_vm_and_opt(&mut vm, source, "<test>", OptimizationLevel::Standard) {
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

/// Run Aelys source code and check if it returns the expected string.
pub fn assert_aelys_str(source: &str, expected: &str) {
    use aelys::new_vm;
    let mut vm = new_vm().expect("Failed to create VM");
    let result = aelys::run_with_vm_and_opt(&mut vm, source, "<test>", OptimizationLevel::Standard)
        .expect("Aelys execution should succeed");
    if let Some(ptr) = result.as_ptr() {
        let heap = vm.heap();
        if let Some(obj) = heap.get(aelys_runtime::vm::GcRef::new(ptr))
            && let aelys_runtime::vm::ObjectKind::String(s) = &obj.kind
        {
            assert_eq!(
                s.as_str(),
                expected,
                "Expected string '{}' but got '{}'",
                expected,
                s.as_str()
            );
            return;
        }
    }
    panic!("Expected string '{}' but got {:?}", expected, result);
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

/// Run Aelys source code, allowing both success and error.
/// Returns Ok(Value) on success, Err(String) on error.
pub fn run_aelys_result(source: &str) -> Result<Value, String> {
    let mut vm = new_vm().expect("Failed to create VM");
    run_with_vm_and_opt(&mut vm, source, "<test>", OptimizationLevel::Standard)
        .map_err(|e| e.to_string())
}

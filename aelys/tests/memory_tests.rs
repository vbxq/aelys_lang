//! Integration tests for manual memory operations.

mod common;
use common::*;

#[test]
fn test_alloc_store_load_free() {
    let code = r#"
let p = alloc(10)
store(p, 0, 42)
store(p, 9, 100)
let a = load(p, 0)
let b = load(p, 9)
free(p)
a + b
"#;
    let result = run_aelys(code);
    assert_eq!(result.as_int(), Some(142)); // 42 + 100
}

#[test]
fn test_double_free_error() {
    let code = r#"
let p = alloc(10)
free(p)
free(p)
"#;
    let err = run_aelys_err(code);
    assert!(
        err.to_lowercase().contains("double free") || err.to_lowercase().contains("freed"),
        "Expected double free error, got: {}",
        err
    );
}

#[test]
fn test_use_after_free_load() {
    let code = r#"
let p = alloc(10)
free(p)
load(p, 0)
"#;
    let err = run_aelys_err(code);
    assert!(
        err.to_lowercase().contains("free") || err.to_lowercase().contains("freed"),
        "Expected use after free error, got: {}",
        err
    );
}

#[test]
fn test_use_after_free_store() {
    let code = r#"
let p = alloc(10)
free(p)
store(p, 0, 1)
"#;
    let err = run_aelys_err(code);
    assert!(
        err.to_lowercase().contains("free") || err.to_lowercase().contains("freed"),
        "Expected use after free error, got: {}",
        err
    );
}

#[test]
fn test_out_of_bounds() {
    let code = r#"
let p = alloc(5)
load(p, 10)
"#;
    let err = run_aelys_err(code);
    assert!(
        err.to_lowercase().contains("bound") || err.to_lowercase().contains("size"),
        "Expected out of bounds error, got: {}",
        err
    );
}

#[test]
fn test_alloc_zero_fails() {
    let code = "let p = alloc(0)";
    let err = run_aelys_err(code);
    assert!(
        err.to_lowercase().contains("size") || err.to_lowercase().contains("allocation"),
        "Expected invalid size error, got: {}",
        err
    );
}

#[test]
fn test_negative_offset() {
    let code = r#"
let p = alloc(10)
load(p, -1)
"#;
    let err = run_aelys_err(code);
    assert!(
        err.to_lowercase().contains("negative") || err.to_lowercase().contains("index"),
        "Expected negative index error, got: {}",
        err
    );
}

#[test]
fn test_free_null_is_noop() {
    let code = r#"
free(null)
42
"#;
    let result = run_aelys(code);
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_multiple_allocations() {
    let code = r#"
let p1 = alloc(5)
let p2 = alloc(10)
let p3 = alloc(3)
store(p1, 0, 1)
store(p2, 0, 2)
store(p3, 0, 3)
let a = load(p1, 0)
let b = load(p2, 0)
let c = load(p3, 0)
free(p1)
free(p2)
free(p3)
a + b + c
"#;
    let result = run_aelys(code);
    assert_eq!(result.as_int(), Some(6)); // 1 + 2 + 3
}

#[test]
fn test_store_different_types() {
    let code = r#"
let p = alloc(10)
store(p, 0, 42)
store(p, 1, true)
store(p, 2, null)
store(p, 3, 3.14)
let a = load(p, 0)
let b = load(p, 1)
let c = load(p, 2)
free(p)
a
"#;
    let result = run_aelys(code);
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_alloc_boundary_last_element() {
    let code = r#"
let p = alloc(5)
store(p, 4, 99)
let val = load(p, 4)
free(p)
val
"#;
    let result = run_aelys(code);
    assert_eq!(result.as_int(), Some(99));
}

#[test]
fn test_store_out_of_bounds() {
    let code = r#"
let p = alloc(5)
store(p, 5, 42)
"#;
    let err = run_aelys_err(code);
    assert!(
        err.to_lowercase().contains("bound") || err.to_lowercase().contains("size"),
        "Expected out of bounds error, got: {}",
        err
    );
}

#[test]
fn test_negative_handle_free() {
    // free(-1) silently ignores negative handles (like C's free(NULL))
    let code = "free(-1)";
    run_aelys_ok(code); // should succeed without error
}

#[test]
fn test_negative_size_alloc() {
    let code = "alloc(-10)";
    let err = run_aelys_err(code);
    assert!(
        err.to_lowercase().contains("non-negative")
            || err.to_lowercase().contains("size")
            || err.to_lowercase().contains("allocation"),
        "Expected invalid size error, got: {}",
        err
    );
}

#[test]
fn test_manual_memory_in_function() {
    let code = r#"
fn test_mem() {
    let p = alloc(3)
    store(p, 0, 10)
    store(p, 1, 20)
    store(p, 2, 30)
    let sum = load(p, 0) + load(p, 1) + load(p, 2)
    free(p)
    return sum
}
test_mem()
"#;
    let result = run_aelys(code);
    assert_eq!(result.as_int(), Some(60)); // 10 + 20 + 30
}

#[test]
fn test_manual_memory_with_loop() {
    let code = r#"
let p = alloc(10)
let mut i = 0
while i < 10 {
    store(p, i, i * 2)
    i++
}
let sum = 0
let mut j = 0
while j < 10 {
    let val = load(p, j)
    j++
}
free(p)
load(p, 0)
"#;
    // This should fail because we're loading after free
    let err = run_aelys_err(code);
    assert!(
        err.to_lowercase().contains("free") || err.to_lowercase().contains("freed"),
        "Expected use after free error, got: {}",
        err
    );
}

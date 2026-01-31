use std::fs::{self, File};
use std::io::Write;
use std::path::PathBuf;
use tempfile::TempDir;

use aelys_driver::run_file;

/// Helper to create a benchmarks directory with module files
fn create_module_env() -> TempDir {
    tempfile::tempdir().expect("Failed to create temp dir")
}

/// Helper to write a file in the temp directory
fn write_file(dir: &TempDir, path: &str, content: &str) -> PathBuf {
    let file_path = dir.path().join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("Failed to create parent directories");
    }
    let mut file = File::create(&file_path).expect("Failed to create file");
    write!(file, "{}", content).expect("Failed to write file");
    file_path
}

// ==Basic Module Import Tests==

#[test]
fn test_basic_module_import() {
    let dir = create_module_env();

    write_file(
        &dir,
        "utils.aelys",
        r#"
pub fn double(x) { x * 2 }
pub let FACTOR = 10
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs utils
needs print from std.io
print(utils.double(5))
utils.FACTOR
"#,
    );

    let result = run_file(&main_path).expect("Module import should succeed");
    assert_eq!(result.as_int(), Some(10));
}

#[test]
fn test_module_import_with_alias() {
    let dir = create_module_env();

    write_file(
        &dir,
        "utilities.aelys",
        r#"
pub fn triple(x) { x * 3 }
pub let VALUE = 42
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs utilities as u
u.triple(7) + u.VALUE
"#,
    );

    let result = run_file(&main_path).expect("Alias import should succeed");
    assert_eq!(result.as_int(), Some(21 + 42)); // 63
}

#[test]
fn test_std_module_import_with_alias() {
    let dir = create_module_env();

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs std.math as m
m.sin(0)
"#,
    );

    let result = run_file(&main_path).expect("Std alias import should succeed");
    assert_eq!(result.as_float(), Some(0.0));
}

#[test]
fn test_std_symbol_import_from() {
    let dir = create_module_env();

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs cos from std.math
cos(0)
"#,
    );

    let result = run_file(&main_path).expect("Std symbol import should succeed");
    assert_eq!(result.as_float(), Some(1.0));
}

#[test]
fn test_wildcard_import() {
    let dir = create_module_env();

    write_file(
        &dir,
        "math.aelys",
        r#"
pub fn square(x) { x * x }
pub let PI = 3
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs math.*
square(5) + PI
"#,
    );

    let result = run_file(&main_path).expect("Wildcard import should succeed");
    assert_eq!(result.as_int(), Some(25 + 3)); // 28
}

#[test]
fn test_specific_symbol_import() {
    let dir = create_module_env();

    write_file(
        &dir,
        "funcs.aelys",
        r#"
pub fn add(a, b) { a + b }
pub fn sub(a, b) { a - b }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs funcs.add
add(10, 5)
"#,
    );

    let result = run_file(&main_path).expect("Symbol import should succeed");
    assert_eq!(result.as_int(), Some(15));
}

// ==Nested Module Tests==

#[test]
fn test_nested_module_path() {
    let dir = create_module_env();

    write_file(
        &dir,
        "helpers/math.aelys",
        r#"
pub fn cube(x) { x * x * x }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs helpers.math
math.cube(3)
"#,
    );

    let result = run_file(&main_path).expect("Nested module should succeed");
    assert_eq!(result.as_int(), Some(27));
}

#[test]
fn test_nested_module_with_mod_aelys() {
    let dir = create_module_env();

    write_file(
        &dir,
        "utils/mod.aelys",
        r#"
pub fn helper() { 100 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs utils
utils.helper()
"#,
    );

    let result = run_file(&main_path).expect("mod.aelys module should succeed");
    assert_eq!(result.as_int(), Some(100));
}

// ==Visibility Tests==

#[test]
fn test_private_function_not_exported() {
    let dir = create_module_env();

    write_file(
        &dir,
        "private_mod.aelys",
        r#"
fn secret() { 42 }
pub fn public() { secret() }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs private_mod
private_mod.public()
"#,
    );

    // Public function that calls private should work
    let result = run_file(&main_path).expect("Public function should succeed");
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_private_function_returns_null() {
    let dir = create_module_env();

    write_file(
        &dir,
        "private_mod.aelys",
        r#"
fn secret() { 42 }
pub fn public() { 1 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs private_mod
private_mod.secret
"#,
    );

    // Private function access returns null (not exported)
    let result = run_file(&main_path).expect("Should succeed but return null");
    assert!(result.is_null());
}

#[test]
fn test_private_let_not_exported() {
    let dir = create_module_env();

    write_file(
        &dir,
        "private_mod.aelys",
        r#"
let secret = 100
pub let public = 200
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs private_mod
private_mod.public
"#,
    );

    let result = run_file(&main_path).expect("Public let should be accessible");
    assert_eq!(result.as_int(), Some(200));
}

// ==Circular Dependency Tests==

#[test]
fn test_circular_dependency_detected() {
    let dir = create_module_env();

    write_file(
        &dir,
        "a.aelys",
        r#"
needs b
pub fn a_func() { 1 }
"#,
    );

    write_file(
        &dir,
        "b.aelys",
        r#"
needs a
pub fn b_func() { 2 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs a
a.a_func()
"#,
    );

    let result = run_file(&main_path);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("circular dependency"),
        "Error should mention circular dependency: {}",
        err
    );
}

#[test]
fn test_self_import_detected() {
    let dir = create_module_env();

    write_file(
        &dir,
        "self_import.aelys",
        r#"
needs self_import
pub fn foo() { 1 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs self_import
self_import.foo()
"#,
    );

    let result = run_file(&main_path);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("circular dependency"),
        "Self-import should be detected: {}",
        err
    );
}

// ==Module Not Found Tests==

#[test]
fn test_module_not_found() {
    let dir = create_module_env();

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs nonexistent
nonexistent.foo()
"#,
    );

    let result = run_file(&main_path);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("module not found") || err.contains("not found"),
        "Error should mention module not found: {}",
        err
    );
}

#[test]
fn test_symbol_not_found_in_module() {
    let dir = create_module_env();

    write_file(
        &dir,
        "utils.aelys",
        r#"
pub fn existing() { 1 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs utils.nonexistent
nonexistent()
"#,
    );

    let result = run_file(&main_path);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("not found") || err.contains("nonexistent"),
        "Error should mention symbol not found: {}",
        err
    );
}

// ==Multiple Imports Tests==

#[test]
fn test_multiple_module_imports() {
    let dir = create_module_env();

    write_file(
        &dir,
        "mod_a.aelys",
        r#"
pub fn a() { 10 }
"#,
    );

    write_file(
        &dir,
        "mod_b.aelys",
        r#"
pub fn b() { 20 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs mod_a
needs mod_b
mod_a.a() + mod_b.b()
"#,
    );

    let result = run_file(&main_path).expect("Multiple imports should succeed");
    assert_eq!(result.as_int(), Some(30));
}

#[test]
fn test_module_import_with_dependencies() {
    let dir = create_module_env();

    write_file(
        &dir,
        "base.aelys",
        r#"
pub fn base_func() { 5 }
"#,
    );

    write_file(
        &dir,
        "derived.aelys",
        r#"
needs base
pub fn derived_func() { base.base_func() * 2 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs derived
derived.derived_func()
"#,
    );

    let result = run_file(&main_path).expect("Module with dependencies should succeed");
    assert_eq!(result.as_int(), Some(10));
}

// ==Edge Cases==

#[test]
fn test_empty_module() {
    let dir = create_module_env();

    write_file(
        &dir,
        "empty.aelys",
        r#"
// Empty module - no exports
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs empty
42
"#,
    );

    let result = run_file(&main_path).expect("Empty module should be allowed");
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_same_module_imported_twice() {
    let dir = create_module_env();

    write_file(
        &dir,
        "utils.aelys",
        r#"
pub let COUNTER = 1
"#,
    );

    write_file(
        &dir,
        "mod_a.aelys",
        r#"
needs utils
pub fn get_counter() { utils.COUNTER }
"#,
    );

    write_file(
        &dir,
        "mod_b.aelys",
        r#"
needs utils
pub fn get_counter_too() { utils.COUNTER }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs mod_a
needs mod_b
mod_a.get_counter() + mod_b.get_counter_too()
"#,
    );

    // Same module should only be loaded once (cached)
    let result = run_file(&main_path).expect("Diamond dependency should work");
    assert_eq!(result.as_int(), Some(2));
}

// ==Direct Import Tests==

#[test]
fn test_std_direct_import() {
    let dir = create_module_env();

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs std.math
sin(0) + cos(0)
"#,
    );

    let result = run_file(&main_path).expect("Direct import should work");
    assert_eq!(result.as_float(), Some(1.0));
}

#[test]
fn test_std_direct_and_qualified() {
    let dir = create_module_env();

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs std.math
sin(0) + math.cos(0)
"#,
    );

    let result = run_file(&main_path).expect("Both forms should work");
    assert_eq!(result.as_float(), Some(1.0));
}

#[test]
fn test_alias_disables_direct_import() {
    let dir = create_module_env();

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs std.math as m
sin(0)
"#,
    );

    let result = run_file(&main_path);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("sin"), "Should fail on direct 'sin' call: {}", err);
}

#[test]
fn test_alias_qualified_works() {
    let dir = create_module_env();

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs std.math as m
m.sin(0) + m.cos(0)
"#,
    );

    let result = run_file(&main_path).expect("Aliased qualified access should work");
    assert_eq!(result.as_float(), Some(1.0));
}

#[test]
fn test_custom_module_direct_import() {
    let dir = create_module_env();

    write_file(
        &dir,
        "mymath.aelys",
        r#"
pub fn double(x) { x * 2 }
pub fn triple(x) { x * 3 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs mymath
double(5) + triple(2)
"#,
    );

    let result = run_file(&main_path).expect("Direct import for custom modules should work");
    assert_eq!(result.as_int(), Some(16));
}

#[test]
fn test_custom_module_alias_qualified_works() {
    let dir = create_module_env();

    write_file(
        &dir,
        "mymath.aelys",
        r#"
pub fn double(x) { x * 2 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs mymath as mm
mm.double(5)
"#,
    );

    let result = run_file(&main_path).expect("Aliased qualified access should work");
    assert_eq!(result.as_int(), Some(10));
}

#[test]
fn test_custom_module_alias_no_direct() {
    let dir = create_module_env();

    write_file(
        &dir,
        "mymath.aelys",
        r#"
pub fn double(x) { x * 2 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs mymath as mm
double(5)
"#,
    );

    let result = run_file(&main_path);
    assert!(result.is_err());
}

#[test]
fn test_symbol_conflict_error() {
    let dir = create_module_env();

    write_file(
        &dir,
        "mod_a.aelys",
        r#"
pub fn shared() { 1 }
"#,
    );

    write_file(
        &dir,
        "mod_b.aelys",
        r#"
pub fn shared() { 2 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs mod_a
needs mod_b
shared()
"#,
    );

    let result = run_file(&main_path);
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("conflict") || err.contains("shared"),
        "Should detect symbol conflict: {}",
        err
    );
}

#[test]
fn test_no_conflict_with_both_aliased() {
    let dir = create_module_env();

    write_file(
        &dir,
        "mod_a.aelys",
        r#"
pub fn shared() { 1 }
"#,
    );

    write_file(
        &dir,
        "mod_b.aelys",
        r#"
pub fn shared() { 2 }
"#,
    );

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs mod_a as a
needs mod_b as b
a.shared() + b.shared()
"#,
    );

    let result = run_file(&main_path).expect("Both aliased should work");
    assert_eq!(result.as_int(), Some(3));
}

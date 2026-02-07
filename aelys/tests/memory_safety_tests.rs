#[test]
fn test_arc_bytecode_pointer_stability() {
    use aelys_runtime::Function;

    let mut func = Function::new(Some("test".to_string()), 0);
    func.num_registers = 2;

    func.set_bytecode(vec![
        0x01_00_00_2A, // LoadI r0, 42
        0x22_00_00_00, // Print r0
        0x21_00_00_00, // Return0
    ]);

    // The bytecode is now an Arc<[u32]>, so taking pointers is safe
    let bytecode_ptr_1 = func.bytecode.as_ptr();

    // Clone the function (Arc clone, not deep copy)
    let func2 = func.clone();
    let bytecode_ptr_2 = func2.bytecode.as_ptr();

    // Pointers should be identical (same allocation)
    assert_eq!(bytecode_ptr_1, bytecode_ptr_2);
}

#[test]
fn test_bytecode_finalization() {
    use aelys_runtime::Function;

    let mut func = Function::new(Some("test".to_string()), 0);

    // Push raw bytecode during compilation
    func.push_raw(0x01_00_00_01); // LoadI r0, 1
    func.push_raw(0x21_00_00_00); // Return0

    // Before finalization, bytecode Arc should be empty
    assert_eq!(func.bytecode.len(), 0);

    // Finalize to transfer builder to immutable Arc
    func.finalize_bytecode();

    // After finalization, bytecode should be populated
    assert_eq!(func.bytecode.len(), 2);
    assert_eq!(func.bytecode[0], 0x01_00_00_01);
    assert_eq!(func.bytecode[1], 0x21_00_00_00);
}

#[test]
fn test_path_traversal_rejection() {
    use aelys_driver::modules::ModuleLoader;
    use aelys_syntax::Source;
    use std::path::PathBuf;

    let source = Source::new("test.aelys", "");
    let entry_path = PathBuf::from("/tmp/test.aelys");
    let loader = ModuleLoader::new(&entry_path, source);

    // Test path traversal attempts - these should all fail
    let malicious_paths = vec![
        vec!["..".to_string(), "etc".to_string(), "passwd".to_string()],
        vec![".".to_string(), "hidden".to_string()],
        vec!["foo/bar".to_string()],  // Contains path separator
        vec!["foo\\bar".to_string()], // Contains backslash
    ];

    for path in malicious_paths {
        let result = loader.resolve_path(&path);
        // Should fail with ModuleNotFound (path validation failed)
        assert!(result.is_err(), "Path {:?} should be rejected", path);
    }
}

#[test]
fn test_valid_module_paths() {
    use aelys_driver::modules::ModuleLoader;
    use aelys_syntax::Source;
    use std::path::PathBuf;

    let source = Source::new("test.aelys", "");
    let entry_path = PathBuf::from("/tmp/test.aelys");
    let loader = ModuleLoader::new(&entry_path, source);

    // Valid paths (just names without path components)
    let valid_paths = vec![
        vec!["utils".to_string()],
        vec!["math".to_string(), "helpers".to_string()],
        vec!["my_module".to_string()],
    ];

    for path in valid_paths {
        let result = loader.resolve_path(&path);
        // These may fail because files don't exist, but they should NOT fail
        // due to path traversal validation (error will be ModuleNotFound with searched_paths)
        if let Err(err) = result {
            let err_str = format!("{}", err);
            // Should have searched_paths (legitimate module not found), not empty (path traversal)
            assert!(
                err_str.contains("searched in:") || !err_str.contains("[]"),
                "Path {:?} should pass validation but got: {}",
                path,
                err_str
            );
        }
    }
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "type confusion")]
fn test_debug_assertion_int_unchecked_on_float() {
    use aelys_runtime::Value;

    let float_val = Value::float(2.72);
    let _ = float_val.as_int_unchecked();
}

#[cfg(debug_assertions)]
#[test]
#[should_panic(expected = "type confusion")]
fn test_debug_assertion_float_unchecked_on_int() {
    use aelys_runtime::Value;

    let int_val = Value::int(42);
    let _ = int_val.as_float_unchecked();
}

#[test]
fn test_correct_type_conversions() {
    use aelys_runtime::Value;

    let int_val = Value::int(42);
    let float_val = Value::float(2.72);

    // These should work without panic
    assert_eq!(int_val.as_int_unchecked(), 42);
    assert!((float_val.as_float_unchecked() - 2.72).abs() < 0.001);
}

#[test]
fn test_nested_function_finalization() {
    use aelys_runtime::Function;

    let mut outer = Function::new(Some("outer".to_string()), 0);
    outer.push_raw(0x01_00_00_01); // LoadI r0, 1

    let mut inner = Function::new(Some("inner".to_string()), 0);
    inner.push_raw(0x01_00_00_02); // LoadI r0, 2
    inner.push_raw(0x21_00_00_00); // Return0

    outer.nested_functions.push(inner);
    outer.push_raw(0x21_00_00_00); // Return0

    outer.finalize_bytecode();

    // Both should be finalized
    assert_eq!(outer.bytecode.len(), 2);
    assert_eq!(outer.nested_functions[0].bytecode.len(), 2);
}

#[test]
fn test_deep_recursion_works() {
    use aelys::run;

    let source = r#"
fn countdown(n) {
    if n <= 0 {
        return 0
    }
    countdown(n - 1)
}

countdown(100)
"#;

    let result = run(source, "test.aelys");
    assert!(result.is_ok(), "Deep recursion should work: {:?}", result);
}

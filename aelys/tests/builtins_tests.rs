use aelys_runtime::{VM, Value, builtin_type, register_builtins};
use aelys_syntax::Source;

fn make_test_vm() -> VM {
    let source = Source::new("test.aelys".to_string(), "".to_string());
    VM::new(source).unwrap()
}

#[test]
fn test_register_builtins() {
    let mut vm = make_test_vm();
    register_builtins(&mut vm).unwrap();

    // Verify core built-ins are registered as globals
    assert!(vm.get_global("type").is_some());
    assert!(vm.get_global("alloc").is_some());
    assert!(vm.get_global("free").is_some());
    assert!(vm.get_global("load").is_some());
    assert!(vm.get_global("store").is_some());
}

#[test]
fn test_builtin_type() {
    let mut vm = make_test_vm();

    // Test type() with different value types
    let result = builtin_type(&mut vm, &[Value::int(42)]).unwrap();
    assert!(result.is_ptr()); // Returns a string

    let result = builtin_type(&mut vm, &[Value::float(3.14)]).unwrap();
    assert!(result.is_ptr());

    let result = builtin_type(&mut vm, &[Value::bool(true)]).unwrap();
    assert!(result.is_ptr());

    let result = builtin_type(&mut vm, &[Value::null()]).unwrap();
    assert!(result.is_ptr());
}

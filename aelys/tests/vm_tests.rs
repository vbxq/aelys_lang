//! Tests for the Aelys VM

use aelys_common::{RuntimeError, RuntimeErrorKind};
use aelys_runtime::{CallFrame, Function, GlobalLayout, MAX_FRAMES, OpCode, VM, Value};
use aelys_syntax::Source;
use std::sync::Arc;

fn make_test_source() -> Arc<Source> {
    Source::new("test.aelys", "fn test() { }")
}

#[test]
fn test_vm_creation() {
    let source = make_test_source();
    let vm = VM::new(source).unwrap();

    assert_eq!(vm.frame_count(), 0);
    // VM now pre-allocates 32768 registers for performance
    assert_eq!(vm.register_count(), 32768);
    assert!(!vm.is_in_no_gc());
}

#[test]
fn test_push_pop_frame() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    let mut func = Function::new(Some("test".to_string()), 0);
    func.num_registers = 5;
    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();

    let frame = CallFrame::new(func_ref, 0, std::ptr::null(), 0, std::ptr::null(), 0, 0);

    vm.push_frame(frame).unwrap();
    assert_eq!(vm.frame_count(), 1);

    let popped = vm.pop_frame();
    assert!(popped.is_some());
    assert_eq!(vm.frame_count(), 0);
}

#[test]
fn test_frame_stack_overflow() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    let mut func = Function::new(Some("test".to_string()), 0);
    func.num_registers = 5;
    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();

    // Push MAX_FRAMES frames
    for _ in 0..MAX_FRAMES {
        let frame = CallFrame::new(func_ref, 0, std::ptr::null(), 0, std::ptr::null(), 0, 0);
        vm.push_frame(frame).unwrap();
    }

    // Next push should fail
    let frame = CallFrame::new(func_ref, 0, std::ptr::null(), 0, std::ptr::null(), 0, 0);
    let result = vm.push_frame(frame);
    assert!(result.is_err());
}

#[test]
fn test_read_write_register() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    let mut func = Function::new(Some("test".to_string()), 0);
    func.num_registers = 10;
    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();

    let frame = CallFrame::new(func_ref, 0, std::ptr::null(), 0, std::ptr::null(), 0, 0);
    vm.push_frame(frame).unwrap();

    // Write and read
    vm.write_register(0, Value::int(42)).unwrap();
    assert_eq!(vm.read_register(0).unwrap().as_int(), Some(42));

    vm.write_register(5, Value::bool(true)).unwrap();
    assert_eq!(vm.read_register(5).unwrap().as_bool(), Some(true));
}

#[test]
fn test_windowed_registers() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // First frame at base 0
    let mut func1 = Function::new(Some("func1".to_string()), 0);
    func1.num_registers = 5;
    let func1_ref = vm.alloc_function(func1).unwrap();

    let frame1 = CallFrame::new(func1_ref, 0, std::ptr::null(), 0, std::ptr::null(), 0, 0);
    vm.push_frame(frame1).unwrap();

    vm.write_register(0, Value::int(100)).unwrap();
    vm.write_register(1, Value::int(200)).unwrap();

    // Second frame at base 5
    let mut func2 = Function::new(Some("func2".to_string()), 0);
    func2.num_registers = 3;
    let func2_ref = vm.alloc_function(func2).unwrap();

    let frame2 = CallFrame::new(func2_ref, 5, std::ptr::null(), 0, std::ptr::null(), 0, 0);
    vm.push_frame(frame2).unwrap();

    // These writes go to different slots
    vm.write_register(0, Value::int(10)).unwrap();
    vm.write_register(1, Value::int(20)).unwrap();

    // Current frame sees 10, 20
    assert_eq!(vm.read_register(0).unwrap().as_int(), Some(10));
    assert_eq!(vm.read_register(1).unwrap().as_int(), Some(20));

    // Pop back to first frame
    vm.pop_frame();

    // First frame still has 100, 200
    assert_eq!(vm.read_register(0).unwrap().as_int(), Some(100));
    assert_eq!(vm.read_register(1).unwrap().as_int(), Some(200));
}

#[test]
fn test_global_variables() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    assert!(vm.get_global("x").is_none());

    vm.set_global("x".to_string(), Value::int(42));
    assert_eq!(vm.get_global("x"), Some(Value::int(42)));

    vm.set_global("x".to_string(), Value::bool(true));
    assert_eq!(vm.get_global("x"), Some(Value::bool(true)));
}

#[test]
fn test_no_gc_depth() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    assert_eq!(vm.no_gc_depth(), 0);
    assert!(!vm.is_in_no_gc());

    vm.enter_no_gc();
    assert_eq!(vm.no_gc_depth(), 1);
    assert!(vm.is_in_no_gc());

    vm.enter_no_gc();
    assert_eq!(vm.no_gc_depth(), 2);
    assert!(vm.is_in_no_gc());

    vm.exit_no_gc();
    assert_eq!(vm.no_gc_depth(), 1);
    assert!(vm.is_in_no_gc());

    vm.exit_no_gc();
    assert_eq!(vm.no_gc_depth(), 0);
    assert!(!vm.is_in_no_gc());

    // Extra exits should saturate at 0
    vm.exit_no_gc();
    assert_eq!(vm.no_gc_depth(), 0);
}

#[test]
fn test_maybe_collect_respects_no_gc() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Allocate strings to fill heap past threshold
    for i in 0..10000 {
        vm.alloc_string(&format!("string_number_{}", i)).unwrap();
    }

    let objects_before = vm.heap().object_count();
    assert!(objects_before > 0);

    // Enter no_gc - collection should be suppressed even if threshold is reached
    vm.enter_no_gc();
    vm.maybe_collect();

    // Objects should not be collected
    assert_eq!(vm.heap().object_count(), objects_before);

    // Exit no_gc and force collection
    vm.exit_no_gc();
    vm.collect(); // Force collection instead of maybe_collect

    // TODO: everytime we add a built-in function, this test needs updating, we should find a better way
    // Now objects should be collected, except for the built-in functions in globals
    // Built-in functions are rooted in globals, so they won't be collected
    // We have 5 built-in functions: type, alloc, free, load, store
    assert_eq!(vm.heap().object_count(), 5);
}

#[test]
fn test_collect_marks_registers() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create some strings
    let str1 = vm.alloc_string("keep me").unwrap();
    let str2 = vm.alloc_string("free me").unwrap();

    // Push a frame and store str1 in a register
    let mut func = Function::new(Some("test".to_string()), 0);
    func.num_registers = 5;
    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();

    // num_registers must match the function's register count for GC to mark them
    let frame = CallFrame::new(func_ref, 0, std::ptr::null(), 0, std::ptr::null(), 0, 5);
    vm.push_frame(frame).unwrap();

    vm.write_register(0, Value::ptr(str1.index())).unwrap();

    // str2 is not rooted anywhere
    // Force collection
    vm.collect();

    // str1 should be kept, str2 should be freed
    assert!(vm.heap().get(str1).is_some());
    assert!(vm.heap().get(str2).is_none());
}

#[test]
fn test_collect_marks_globals() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    let str1 = vm.alloc_string("global string").unwrap();
    let str2 = vm.alloc_string("unreachable").unwrap();

    vm.set_global("my_str".to_string(), Value::ptr(str1.index()));

    vm.collect();

    assert!(vm.heap().get(str1).is_some());
    assert!(vm.heap().get(str2).is_none());
}

#[test]
fn test_runtime_error_with_stack_trace() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    let mut func = Function::new(Some("test_func".to_string()), 2);
    func.num_registers = 10;
    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();

    let frame = CallFrame::new(func_ref, 0, std::ptr::null(), 0, std::ptr::null(), 0, 0);
    vm.push_frame(frame).unwrap();

    let error = vm.runtime_error(RuntimeErrorKind::DivisionByZero);

    assert!(matches!(error.kind, RuntimeErrorKind::DivisionByZero));
    assert!(!error.stack_trace.is_empty());
}
#[test]
fn test_current_frame_methods() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    let mut func = Function::new(Some("test".to_string()), 0);
    func.num_registers = 5;
    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();

    let frame = CallFrame::new(func_ref, 0, std::ptr::null(), 0, std::ptr::null(), 0, 0);
    vm.push_frame(frame).unwrap();

    assert_eq!(vm.current_frame().unwrap().ip(), 0);

    vm.current_frame_mut().unwrap().advance_ip();
    assert_eq!(vm.current_frame().unwrap().ip(), 1);

    vm.current_frame_mut().unwrap().set_ip(42);
    assert_eq!(vm.current_frame().unwrap().ip(), 42);
}

// VM Execution Tests

#[test]
fn test_execute_simple_return() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create a function that returns 42
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 2;
    func.emit_b(OpCode::LoadI, 0, 42, 1); // r0 = 42
    func.emit_a(OpCode::Return, 0, 0, 0, 1); // return r0

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();

    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_execute_arithmetic() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create a function that computes 10 + 20 - 5
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 5;
    func.emit_b(OpCode::LoadI, 0, 10, 1); // r0 = 10
    func.emit_b(OpCode::LoadI, 1, 20, 1); // r1 = 20
    func.emit_a(OpCode::Add, 2, 0, 1, 1); // r2 = r0 + r1 (30)
    func.emit_b(OpCode::LoadI, 3, 5, 1); // r3 = 5
    func.emit_a(OpCode::Sub, 4, 2, 3, 1); // r4 = r2 - r3 (25)
    func.emit_a(OpCode::Return, 4, 0, 0, 1); // return r4

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();

    assert_eq!(result.as_int(), Some(25));
}

#[test]
fn test_execute_multiplication_division() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create a function that computes (6 * 7) / 2
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 5;
    func.emit_b(OpCode::LoadI, 0, 6, 1); // r0 = 6
    func.emit_b(OpCode::LoadI, 1, 7, 1); // r1 = 7
    func.emit_a(OpCode::Mul, 2, 0, 1, 1); // r2 = r0 * r1 (42)
    func.emit_b(OpCode::LoadI, 3, 2, 1); // r3 = 2
    func.emit_a(OpCode::Div, 4, 2, 3, 1); // r4 = r2 / r3 (21)
    func.emit_a(OpCode::Return, 4, 0, 0, 1); // return r4

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();

    assert_eq!(result.as_int(), Some(21));
}

#[test]
fn test_execute_modulo() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create a function that computes 17 % 5
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 3;
    func.emit_b(OpCode::LoadI, 0, 17, 1); // r0 = 17
    func.emit_b(OpCode::LoadI, 1, 5, 1); // r1 = 5
    func.emit_a(OpCode::Mod, 2, 0, 1, 1); // r2 = r0 % r1 (2)
    func.emit_a(OpCode::Return, 2, 0, 0, 1); // return r2

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();

    assert_eq!(result.as_int(), Some(2));
}

#[test]
fn test_execute_negation() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create a function that computes -42
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 2;
    func.emit_b(OpCode::LoadI, 0, 42, 1); // r0 = 42
    func.emit_a(OpCode::Neg, 1, 0, 0, 1); // r1 = -r0
    func.emit_a(OpCode::Return, 1, 0, 0, 1); // return r1

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();

    assert_eq!(result.as_int(), Some(-42));
}

#[test]
fn test_execute_division_by_zero() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create a function that divides by zero
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 3;
    func.emit_b(OpCode::LoadI, 0, 10, 1); // r0 = 10
    func.emit_b(OpCode::LoadI, 1, 0, 1); // r1 = 0
    func.emit_a(OpCode::Div, 2, 0, 1, 1); // r2 = r0 / r1 (error!)
    func.emit_a(OpCode::Return, 2, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref);

    assert!(result.is_err());
    if let Err(e) = result {
        assert!(matches!(e.kind, RuntimeErrorKind::DivisionByZero));
    }
}

#[test]
fn test_execute_comparison_operators() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Test: 10 < 20
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 3;
    func.emit_b(OpCode::LoadI, 0, 10, 1); // r0 = 10
    func.emit_b(OpCode::LoadI, 1, 20, 1); // r1 = 20
    func.emit_a(OpCode::Lt, 2, 0, 1, 1); // r2 = r0 < r1
    func.emit_a(OpCode::Return, 2, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_execute_equality() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Test: 42 == 42
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 3;
    func.emit_b(OpCode::LoadI, 0, 42, 1); // r0 = 42
    func.emit_b(OpCode::LoadI, 1, 42, 1); // r1 = 42
    func.emit_a(OpCode::Eq, 2, 0, 1, 1); // r2 = r0 == r1
    func.emit_a(OpCode::Return, 2, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_execute_not_equal() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Test: 10 != 20
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 3;
    func.emit_b(OpCode::LoadI, 0, 10, 1); // r0 = 10
    func.emit_b(OpCode::LoadI, 1, 20, 1); // r1 = 20
    func.emit_a(OpCode::Ne, 2, 0, 1, 1); // r2 = r0 != r1
    func.emit_a(OpCode::Return, 2, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_execute_logical_not() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Test: !true
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 2;
    func.emit_a(OpCode::LoadBool, 0, 1, 0, 1); // r0 = true
    func.emit_a(OpCode::Not, 1, 0, 0, 1); // r1 = !r0
    func.emit_a(OpCode::Return, 1, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_execute_jump() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Test unconditional jump: skip setting r0 to 99
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 1;
    func.emit_b(OpCode::Jump, 0, 2, 1); // jump forward 2 instructions
    func.emit_b(OpCode::LoadI, 0, 99, 1); // r0 = 99 (skipped)
    func.emit_b(OpCode::LoadI, 0, 99, 1); // r0 = 99 (skipped)
    func.emit_b(OpCode::LoadI, 0, 42, 1); // r0 = 42 (executed)
    func.emit_a(OpCode::Return, 0, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_execute_jump_if() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Test conditional jump: if true, jump
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 2;
    func.emit_a(OpCode::LoadBool, 0, 1, 0, 1); // r0 = true
    func.emit_b(OpCode::JumpIf, 0, 1, 1); // if r0, jump forward 1
    func.emit_b(OpCode::LoadI, 1, 99, 1); // r1 = 99 (skipped)
    func.emit_b(OpCode::LoadI, 1, 42, 1); // r1 = 42 (executed)
    func.emit_a(OpCode::Return, 1, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_execute_jump_if_not() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Test conditional jump: if not false, jump
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 2;
    func.emit_a(OpCode::LoadBool, 0, 0, 0, 1); // r0 = false
    func.emit_b(OpCode::JumpIfNot, 0, 1, 1); // if !r0, jump forward 1
    func.emit_b(OpCode::LoadI, 1, 99, 1); // r1 = 99 (skipped)
    func.emit_b(OpCode::LoadI, 1, 42, 1); // r1 = 42 (executed)
    func.emit_a(OpCode::Return, 1, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_execute_load_constant() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Test loading a constant
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 1;
    let k = func.add_constant(Value::int(12345));
    func.emit_a(OpCode::LoadK, 0, k as u8, 0, 1); // r0 = constants[k]
    func.emit_a(OpCode::Return, 0, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_int(), Some(12345));
}

#[test]
fn test_execute_load_null() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 1;
    func.emit_a(OpCode::LoadNull, 0, 0, 0, 1);
    func.emit_a(OpCode::Return, 0, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert!(result.is_null());
}

#[test]
fn test_execute_load_bool() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 1;
    func.emit_a(OpCode::LoadBool, 0, 1, 0, 1); // r0 = true
    func.emit_a(OpCode::Return, 0, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_execute_move() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 2;
    func.emit_b(OpCode::LoadI, 0, 42, 1); // r0 = 42
    func.emit_a(OpCode::Move, 1, 0, 0, 1); // r1 = r0
    func.emit_a(OpCode::Return, 1, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_execute_global_variables() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Test setting and getting global variables
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 2;

    // Create a string constant for the variable name
    let name_str = vm.heap_mut().intern_string("myvar");
    let k = func.add_constant(Value::ptr(name_str.index()));

    func.emit_b(OpCode::LoadI, 0, 123, 1); // r0 = 123
    func.emit_a(OpCode::SetGlobal, 0, k as u8, 0, 1); // myvar = r0
    func.emit_a(OpCode::GetGlobal, 1, k as u8, 0, 1); // r1 = myvar
    func.emit_a(OpCode::Return, 1, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_int(), Some(123));
}

#[test]
fn test_execute_native_function_call() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Define a native function that adds two numbers
    fn add_native(_vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
        let a = args[0].as_int().unwrap_or(0);
        let b = args[1].as_int().unwrap_or(0);
        Ok(Value::int(a + b))
    }

    // Allocate native function
    let native_ref = vm.alloc_native("add", 2, add_native).unwrap();

    // Create bytecode function that calls the native function
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 5;

    // Add the native function as a constant first
    let k = func.add_constant(Value::ptr(native_ref.index()));

    func.emit_a(OpCode::LoadK, 0, k as u8, 0, 1); // r0 = native function
    func.emit_b(OpCode::LoadI, 1, 10, 1); // r1 = 10 (arg1)
    func.emit_b(OpCode::LoadI, 2, 20, 1); // r2 = 20 (arg2)
    func.emit_c(OpCode::Call, 3, 0, 2, 1); // r3 = r0(r1, r2)
    func.emit_a(OpCode::Return, 3, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_int(), Some(30));
}

#[test]
fn test_execute_return0() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Test Return0 (returns null)
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 1;
    func.emit_b(OpCode::LoadI, 0, 42, 1); // r0 = 42
    func.emit_a(OpCode::Return0, 0, 0, 0, 1); // return null (ignore r0)

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert!(result.is_null());
}

#[test]
fn test_execute_no_gc_control() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Test EnterNoGc and ExitNoGc
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 1;
    func.emit_a(OpCode::EnterNoGc, 0, 0, 0, 1);
    func.emit_a(OpCode::ExitNoGc, 0, 0, 0, 1);
    func.emit_b(OpCode::LoadI, 0, 42, 1);
    func.emit_a(OpCode::Return, 0, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();
    assert_eq!(result.as_int(), Some(42));
    assert!(!vm.is_in_no_gc());
}

#[test]
fn test_type_error_add_incompatible() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Try to add null + int
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 3;
    func.emit_a(OpCode::LoadNull, 0, 0, 0, 1);
    func.emit_b(OpCode::LoadI, 1, 10, 1);
    func.emit_a(OpCode::Add, 2, 0, 1, 1);
    func.emit_a(OpCode::Return, 2, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref);
    assert!(result.is_err());
}

#[test]
fn test_globals_survive_gc() {
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Set a global value
    vm.set_global("test_var".to_string(), Value::int(42));

    // Force a GC collection
    vm.collect();

    // Globals should survive GC and remain accessible
    let value = vm.get_global("test_var");
    assert!(value.is_some());
    assert_eq!(value.unwrap().as_int(), Some(42));
}

#[test]
fn test_callglobal_native_function() {
    // Test that CallGlobal works with native functions (like type, alloc)
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create a simple function that calls type(42) using CallGlobal
    let mut func = Function::new(Some("main".to_string()), 0);
    func.num_registers = 3;
    func.global_layout = GlobalLayout::new(vec!["type".to_string()]);

    // r1 = 42, then r0 = type(r1)
    func.emit_b(OpCode::LoadI, 1, 42, 1);
    // CallGlobal r0, 0 (global_idx=0=type), 1 (nargs=1)
    // Arguments start at r0+1=r1
    func.emit_a(OpCode::CallGlobal, 0, 0, 1, 1);
    func.push_raw(0); // cache_word_1
    func.push_raw(0); // cache_word_2 with slot_id=0
    func.emit_a(OpCode::Return, 0, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref);

    // type(42) returns a string "int"
    assert!(result.is_ok());
    let value = result.unwrap();
    assert!(
        value.is_ptr(),
        "type() should return a string (ptr), got {:?}",
        value
    );
}

#[test]
fn test_callglobal_user_defined_function() {
    // Test that CallGlobal works with user-defined functions
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create a simple "add" function that adds two numbers
    let mut add_func = Function::new(Some("add".to_string()), 2);
    add_func.num_registers = 3;
    add_func.emit_a(OpCode::Add, 2, 0, 1, 1); // r2 = r0 + r1
    add_func.emit_a(OpCode::Return, 2, 0, 0, 1);

    add_func.finalize_bytecode();
    add_func.finalize_bytecode();
    let add_func_ref = vm.alloc_function(add_func).unwrap();

    // Create a main function that calls add(10, 20) using CallGlobal
    let mut main_func = Function::new(Some("main".to_string()), 0);
    main_func.num_registers = 4;
    main_func.global_layout = GlobalLayout::new(vec!["add".to_string()]);

    // Store add function in globals_by_index and globals hashmap
    vm.set_global_by_index(0, Value::ptr(add_func_ref.index()));
    vm.set_global("add".to_string(), Value::ptr(add_func_ref.index()));

    // Setup arguments: r1 = 10, r2 = 20
    main_func.emit_b(OpCode::LoadI, 1, 10, 1); // r1 = 10
    main_func.emit_b(OpCode::LoadI, 2, 20, 1); // r2 = 20
    // CallGlobal r0, 0 (global_idx=0=add), 2 (nargs=2)
    main_func.emit_a(OpCode::CallGlobal, 0, 0, 2, 1);
    main_func.push_raw(0); // cache_word_1
    main_func.push_raw(0); // cache_word_2 with slot_id=0
    main_func.emit_a(OpCode::Return, 0, 0, 0, 1);

    main_func.finalize_bytecode();
    main_func.finalize_bytecode();
    let main_func_ref = vm.alloc_function(main_func).unwrap();
    let result = vm.execute(main_func_ref);

    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.as_int(), Some(30), "add(10, 20) should return 30");
}

#[test]
fn test_callglobal_recursive_function() {
    // Test that CallGlobal works with recursive functions (inline cache hit)
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create a recursive factorial function
    // fn fact(n) { if n < 2 { return 1 } return n * fact(n - 1) }
    let mut fact_func = Function::new(Some("fact".to_string()), 1);
    fact_func.num_registers = 5;
    fact_func.global_layout = GlobalLayout::new(vec!["fact".to_string()]);

    // if n < 2
    fact_func.emit_b(OpCode::LoadI, 1, 2, 1); // r1 = 2
    fact_func.emit_a(OpCode::Lt, 2, 0, 1, 1); // r2 = r0 < r1 (n < 2)
    fact_func.emit_b(OpCode::JumpIfNot, 2, 2, 1); // if not (n < 2), skip 2 instructions

    // return 1
    fact_func.emit_b(OpCode::LoadI, 3, 1, 1); // r3 = 1
    fact_func.emit_a(OpCode::Return, 3, 0, 0, 1); // return 1

    // return n * fact(n - 1)
    fact_func.emit_a(OpCode::SubI, 2, 0, 1, 1); // r2 = n - 1
    fact_func.emit_a(OpCode::CallGlobal, 3, 0, 1, 1); // r3 = fact(r2) - args at r3+1=r4, so we need to put r2 in the right place

    // Actually, let me redo this more carefully
    // For CallGlobal r3, 0, 1: result in r3, global_idx=0, nargs=1
    // Arguments should be at r3+1 = r4
    // So we need to load r4 = n - 1
    let mut fact_func = Function::new(Some("fact".to_string()), 1);
    fact_func.num_registers = 6;
    fact_func.global_layout = GlobalLayout::new(vec!["fact".to_string()]);

    // if n < 2
    fact_func.emit_b(OpCode::LoadI, 1, 2, 1); // r1 = 2
    fact_func.emit_a(OpCode::Lt, 2, 0, 1, 1); // r2 = r0 < r1 (n < 2)
    fact_func.emit_b(OpCode::JumpIfNot, 2, 2, 1); // if not (n < 2), skip 2 instructions

    // return 1
    fact_func.emit_b(OpCode::LoadI, 3, 1, 1); // r3 = 1
    fact_func.emit_a(OpCode::Return, 3, 0, 0, 1); // return 1

    // return n * fact(n - 1)
    // For CallGlobal with dest=3, args must be at r4
    fact_func.emit_a(OpCode::SubI, 4, 0, 1, 1); // r4 = n - 1
    fact_func.emit_a(OpCode::CallGlobal, 3, 0, 1, 1); // r3 = fact(r4) where args at r3+1=r4
    fact_func.push_raw(0); // cache_word_1
    fact_func.push_raw(0); // cache_word_2 with slot_id=0
    fact_func.emit_a(OpCode::Mul, 5, 0, 3, 1); // r5 = n * r3
    fact_func.emit_a(OpCode::Return, 5, 0, 0, 1); // return r5

    fact_func.finalize_bytecode();
    let fact_func_ref = vm.alloc_function(fact_func).unwrap();
    vm.set_global_by_index(0, Value::ptr(fact_func_ref.index()));
    vm.set_global("fact".to_string(), Value::ptr(fact_func_ref.index()));

    // Create main function that calls fact(5)
    let mut main_func = Function::new(Some("main".to_string()), 0);
    main_func.num_registers = 3;
    main_func.global_layout = GlobalLayout::new(vec!["fact".to_string()]);

    main_func.emit_b(OpCode::LoadI, 1, 5, 1); // r1 = 5 (argument at dest+1=r0+1=r1)
    main_func.emit_a(OpCode::CallGlobal, 0, 0, 1, 1); // r0 = fact(5)
    main_func.push_raw(0); // cache_word_1
    main_func.push_raw(1); // cache_word_2 with slot_id=1 (different from fact's slot)
    main_func.emit_a(OpCode::Return, 0, 0, 0, 1);

    main_func.finalize_bytecode();
    let main_func_ref = vm.alloc_function(main_func).unwrap();
    let result = vm.execute(main_func_ref);

    assert!(result.is_ok());
    let value = result.unwrap();
    assert_eq!(value.as_int(), Some(120), "fact(5) should return 120");
}

#[test]
fn test_callglobal_arity_mismatch() {
    // Test that CallGlobal properly reports arity mismatches
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create a function that takes 2 arguments
    let mut add_func = Function::new(Some("add".to_string()), 2);
    add_func.num_registers = 3;
    add_func.emit_a(OpCode::Add, 2, 0, 1, 1);
    add_func.emit_a(OpCode::Return, 2, 0, 0, 1);

    add_func.finalize_bytecode();
    let add_func_ref = vm.alloc_function(add_func).unwrap();
    vm.set_global_by_index(0, Value::ptr(add_func_ref.index()));
    vm.set_global("add".to_string(), Value::ptr(add_func_ref.index()));

    // Create main function that calls add with wrong number of args
    let mut main_func = Function::new(Some("main".to_string()), 0);
    main_func.num_registers = 3;
    main_func.global_layout = GlobalLayout::new(vec!["add".to_string()]);

    main_func.emit_b(OpCode::LoadI, 1, 10, 1); // Only provide 1 arg
    main_func.emit_a(OpCode::CallGlobal, 0, 0, 1, 1); // Call with nargs=1, but add expects 2
    main_func.push_raw(0); // cache_word_1
    main_func.push_raw(0); // cache_word_2 with slot_id=0
    main_func.emit_a(OpCode::Return, 0, 0, 0, 1);

    main_func.finalize_bytecode();
    let main_func_ref = vm.alloc_function(main_func).unwrap();
    let result = vm.execute(main_func_ref);

    assert!(result.is_err(), "Should fail with arity mismatch");
    if let Err(err) = result {
        assert!(matches!(
            err.kind,
            RuntimeErrorKind::ArityMismatch {
                expected: 2,
                got: 1
            }
        ));
    }
}

#[test]
fn test_callglobal_cache_invalidation_on_gc() {
    // Test that the CallGlobal cache is properly invalidated after GC
    let source = make_test_source();
    let mut vm = VM::new(source).unwrap();

    // Create a function
    let mut func = Function::new(Some("test".to_string()), 0);
    func.num_registers = 1;
    func.emit_b(OpCode::LoadI, 0, 42, 1);
    func.emit_a(OpCode::Return, 0, 0, 0, 1);

    func.finalize_bytecode();
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    // Store in both globals HashMap (for GC roots) and globals_by_index
    vm.set_global("test".to_string(), Value::ptr(func_ref.index()));
    vm.set_global_by_index(0, Value::ptr(func_ref.index()));

    // Create main that calls the function
    let mut main_func = Function::new(Some("main".to_string()), 0);
    main_func.num_registers = 2;
    main_func.global_layout = GlobalLayout::new(vec!["test".to_string()]);

    main_func.emit_a(OpCode::CallGlobal, 0, 0, 0, 1);
    main_func.push_raw(0); // cache_word_1
    main_func.push_raw(0); // cache_word_2 with slot_id=0
    main_func.emit_a(OpCode::Return, 0, 0, 0, 1);

    main_func.finalize_bytecode();
    let main_func_ref = vm.alloc_function(main_func).unwrap();

    // Execute once to populate cache
    let result1 = vm.execute(main_func_ref);
    assert!(result1.is_ok());
    assert_eq!(result1.unwrap().as_int(), Some(42));

    // Force GC - this should clear the call_global_cache
    vm.collect();

    // Execute again - should work even after cache invalidation
    // The cache should be re-populated on the next call
    let mut main_func2 = Function::new(Some("main".to_string()), 0);
    main_func2.num_registers = 2;
    main_func2.global_layout = GlobalLayout::new(vec!["test".to_string()]);
    main_func2.emit_a(OpCode::CallGlobal, 0, 0, 0, 1);
    main_func2.push_raw(0); // cache_word_1
    main_func2.push_raw(1); // cache_word_2 with slot_id=1
    main_func2.emit_a(OpCode::Return, 0, 0, 0, 1);
    main_func2.finalize_bytecode();
    let main_func_ref2 = vm.alloc_function(main_func2).unwrap();

    let result2 = vm.execute(main_func_ref2);
    assert!(
        result2.is_ok(),
        "Execution should succeed after GC: {:?}",
        result2
    );
    assert_eq!(result2.unwrap().as_int(), Some(42));
}

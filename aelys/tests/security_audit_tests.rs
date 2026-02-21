use aelys_bytecode::asm::{BinaryError, deserialize};
use aelys_common::{AelysError, CompileErrorKind, RuntimeErrorKind};
use aelys_driver::run_file;
use aelys_runtime::{Function, GlobalLayout, OpCode, VM, Value};
use aelys_syntax::Source;

fn make_vm() -> VM {
    VM::new(Source::new("test.aelys", "")).unwrap()
}

fn run_function(vm: &mut VM, mut func: Function) -> Result<Value, aelys_common::RuntimeError> {
    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func)?;
    vm.execute(func_ref)
}

#[test]
fn verify_rejects_register_oob() {
    let mut vm = make_vm();
    let mut func = Function::new(Some("oob".to_string()), 0);
    func.num_registers = 1;
    func.emit_a(OpCode::Move, 1, 0, 0, 1);
    func.emit_a(OpCode::Return0, 0, 0, 0, 1);

    func.finalize_bytecode();
    func.num_registers = 1;
    let func_ref = vm.alloc_function(func).unwrap();
    let err = vm.execute(func_ref).unwrap_err();
    match err.kind {
        RuntimeErrorKind::InvalidBytecode(msg) => assert!(msg.contains("register")),
        _ => panic!("expected InvalidBytecode for register OOB"),
    }
}

#[test]
fn verify_rejects_constant_oob() {
    let mut vm = make_vm();
    let mut func = Function::new(Some("const_oob".to_string()), 0);
    func.num_registers = 1;
    func.emit_b(OpCode::LoadK, 0, 1, 1);
    func.emit_a(OpCode::Return0, 0, 0, 0, 1);

    func.finalize_bytecode();
    func.num_registers = 1;
    let func_ref = vm.alloc_function(func).unwrap();
    let err = vm.execute(func_ref).unwrap_err();
    match err.kind {
        RuntimeErrorKind::InvalidBytecode(msg) => assert!(msg.contains("constant")),
        _ => panic!("expected InvalidBytecode for constant OOB"),
    }
}

#[test]
fn manual_heap_invalid_handle_is_rejected() {
    let mut vm = make_vm();
    let mut func = Function::new(Some("heap_bad_handle".to_string()), 0);
    func.num_registers = 3;
    func.emit_b(OpCode::LoadI, 0, 123, 1);
    func.emit_b(OpCode::LoadI, 1, 0, 1);
    func.emit_a(OpCode::LoadMem, 2, 0, 1, 1);
    func.emit_a(OpCode::Return, 2, 0, 0, 1);

    let err = run_function(&mut vm, func).unwrap_err();
    match err.kind {
        RuntimeErrorKind::InvalidMemoryHandle => {}
        _ => panic!("expected InvalidMemoryHandle for invalid manual heap handle"),
    }
}

#[test]
fn verifier_blocks_gc_untracked_registers() {
    let mut vm = make_vm();
    let str_ref = vm.alloc_string("secret").unwrap();

    let mut func = Function::new(Some("gc_oob".to_string()), 0);
    func.num_registers = 1;
    func.constants.push(Value::ptr(str_ref.index()));
    func.emit_b(OpCode::LoadK, 1, 0, 1);
    func.emit_a(OpCode::Return0, 0, 0, 0, 1);

    func.finalize_bytecode();
    func.num_registers = 1;
    let func_ref = vm.alloc_function(func).unwrap();
    let err = vm.execute(func_ref).unwrap_err();
    match err.kind {
        RuntimeErrorKind::InvalidBytecode(msg) => assert!(msg.contains("register")),
        _ => panic!("expected InvalidBytecode for GC register OOB"),
    }
}

#[test]
fn stdlib_exec_denied_without_capability() {
    let src = r#"
needs exec from std.sys
exec("echo hi")
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cap.aelys");
    std::fs::write(&path, src).unwrap();
    let err = run_file(&path).unwrap_err();
    match err {
        AelysError::Runtime(runtime) => match runtime.kind {
            RuntimeErrorKind::CapabilityDenied { operation } => {
                assert_eq!(operation, "sys.exec");
            }
            _ => panic!("expected CapabilityDenied for sys.exec"),
        },
        _ => panic!("expected runtime error for sys.exec"),
    }
}

#[test]
fn verify_rejects_invalid_opcode() {
    let mut vm = make_vm();
    let mut func = Function::new(Some("bad_opcode".to_string()), 0);
    func.num_registers = 0;
    func.set_bytecode(vec![0xFF00_0000]);

    let err = run_function(&mut vm, func).unwrap_err();
    match err.kind {
        RuntimeErrorKind::InvalidBytecode(msg) => assert!(msg.contains("invalid opcode")),
        _ => panic!("expected InvalidBytecode for invalid opcode"),
    }
}

#[test]
fn binary_deserialize_enforces_bytecode_limit() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"VBXQ");
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes()); // name len
    bytes.push(0u8); // arity
    bytes.push(0u8); // num_registers
    bytes.extend_from_slice(&0u16.to_le_bytes()); // constants
    bytes.extend_from_slice(&(1_000_001u32).to_le_bytes()); // bytecode length

    match deserialize(&bytes) {
        Err(BinaryError::LimitExceeded { what, .. }) => {
            assert_eq!(what, "bytecode length");
        }
        Err(other) => panic!("expected LimitExceeded, got {:?}", other),
        Ok(_) => panic!("expected failure for oversized bytecode"),
    }
}

#[test]
fn no_gc_underflow_is_rejected() {
    let mut vm = make_vm();
    let mut func = Function::new(Some("no_gc_underflow".to_string()), 0);
    func.num_registers = 0;
    func.emit_a(OpCode::ExitNoGc, 0, 0, 0, 1);
    func.emit_a(OpCode::Return0, 0, 0, 0, 1);

    let err = run_function(&mut vm, func).unwrap_err();
    match err.kind {
        RuntimeErrorKind::InvalidBytecode(msg) => assert!(msg.contains("no_gc underflow")),
        _ => panic!("expected InvalidBytecode for no_gc underflow"),
    }
}

#[test]
fn nan_is_not_treated_as_pointer() {
    let value = Value::float(f64::NAN);
    assert!(value.is_float());
    assert!(value.as_float().unwrap().is_nan());
    assert!(value.as_ptr().is_none());
}

#[test]
fn read_register_without_frame_is_error() {
    let vm = make_vm();
    let err = vm.read_register(0).unwrap_err();
    match err.kind {
        RuntimeErrorKind::InvalidBytecode(msg) => assert!(msg.contains("no call frame")),
        _ => panic!("expected InvalidBytecode for missing frame"),
    }
}

#[cfg(unix)]
#[test]
fn module_loader_rejects_symlink_escape() {
    use aelys_common::CompileErrorKind;
    use aelys_driver::modules::ModuleLoader;
    use std::fs;
    use std::os::unix::fs::symlink;
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let base = dir.path().join("root");
    let outside = dir.path().join("outside");
    fs::create_dir_all(&base).unwrap();
    fs::create_dir_all(&outside).unwrap();

    let entry = base.join("main.aelys");
    fs::write(&entry, "needs evil.secret").unwrap();

    let secret = outside.join("secret.aelys");
    fs::write(&secret, "pub let X = 1").unwrap();

    let link = base.join("evil");
    symlink(&outside, &link).unwrap();

    let source = Source::new(entry.to_str().unwrap(), "");
    let loader = ModuleLoader::new(&entry, source);
    let err = loader
        .resolve_path(&["evil".to_string(), "secret".to_string()])
        .unwrap_err();

    match err {
        AelysError::Compile(err) => match err.kind {
            CompileErrorKind::ModuleNotFound { .. } => {}
            _ => panic!("expected ModuleNotFound for symlink escape"),
        },
        _ => panic!("expected compile error for symlink escape"),
    }
}

#[test]
fn global_mapping_id_ignores_layout_hash_collisions() {
    let mut vm = make_vm();
    let mut func_a = Function::new(Some("a".to_string()), 0);
    func_a.global_layout = GlobalLayout::new(vec!["first".to_string()]);
    func_a.global_layout_hash = 1;
    let func_a_ref = vm.alloc_function(func_a).unwrap();

    let mut func_b = Function::new(Some("b".to_string()), 0);
    func_b.global_layout = GlobalLayout::new(vec!["second".to_string()]);
    func_b.global_layout_hash = 1;
    let func_b_ref = vm.alloc_function(func_b).unwrap();

    let id_a = vm.get_global_mapping_id(func_a_ref);
    let id_b = vm.get_global_mapping_id(func_b_ref);
    assert_ne!(id_a, 0);
    assert_ne!(id_b, 0);
    assert_ne!(id_a, id_b);
}

#[test]
#[ignore]
fn parser_rejects_deep_expression_recursion() {
    let result = std::thread::Builder::new()
        .stack_size(32 * 1024 * 1024) // 32MB stack to handle deep parsing
        .spawn(|| {
            let depth = 1010;
            let mut code = "(".repeat(depth);
            code.push('1');
            code.push_str(&")".repeat(depth));

            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("recursion_test.aelys");
            std::fs::write(&path, &code).unwrap();

            run_file(&path)
        })
        .unwrap()
        .join()
        .unwrap();

    match result {
        Err(AelysError::Compile(e)) => {
            assert!(
                matches!(e.kind, CompileErrorKind::RecursionDepthExceeded { .. }),
                "expected RecursionDepthExceeded, got {:?}",
                e.kind
            );
        }
        Err(e) => panic!("expected RecursionDepthExceeded compile error, got {:?}", e),
        Ok(_) => panic!("expected compile error for deep recursion"),
    }
}
#[test]
fn lexer_rejects_deep_comment_nesting() {
    let depth = 300;
    let mut code = "/* ".repeat(depth);
    code.push_str(&" */".repeat(depth));

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("comment_test.aelys");
    std::fs::write(&path, &code).unwrap();

    let err = run_file(&path).unwrap_err();
    match err {
        AelysError::Compile(e) => {
            assert!(
                matches!(e.kind, CompileErrorKind::CommentNestingTooDeep { .. }),
                "expected CommentNestingTooDeep, got {:?}",
                e.kind
            );
        }
        _ => panic!("expected compile error for deep comment nesting"),
    }
}

#[test]
fn fs_join_rejects_absolute_path() {
    let src = r#"
needs std.fs
fs.join("/app", "/etc/passwd")
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("path_traversal.aelys");
    std::fs::write(&path, src).unwrap();

    let mut config = aelys_runtime::VmConfig::default();
    config.capabilities.allow_fs = true;

    let err = aelys_driver::run_file_with_config(&path, config, Vec::new()).unwrap_err();
    match err {
        AelysError::Runtime(runtime) => {
            let msg = format!("{:?}", runtime.kind);
            assert!(
                msg.contains("absolute") || msg.contains("path"),
                "expected path traversal error, got: {}",
                msg
            );
        }
        _ => panic!("expected runtime error for path traversal"),
    }
}

#[test]
fn fs_join_rejects_parent_escape() {
    let src = r#"
needs std.fs
fs.join("/app/data", "../../../etc/passwd")
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("path_escape.aelys");
    std::fs::write(&path, src).unwrap();

    let mut config = aelys_runtime::VmConfig::default();
    config.capabilities.allow_fs = true;

    let err = aelys_driver::run_file_with_config(&path, config, Vec::new()).unwrap_err();
    match err {
        AelysError::Runtime(runtime) => {
            let msg = format!("{:?}", runtime.kind);
            assert!(
                msg.contains("escapes") || msg.contains("base"),
                "expected path escape error, got: {}",
                msg
            );
        }
        _ => panic!("expected runtime error for path escape"),
    }
}

#[test]
fn fs_read_bytes_rejects_huge_buffer() {
    let dir = tempfile::tempdir().unwrap();
    let test_file = dir.path().join("test.bin");
    std::fs::write(&test_file, "test content").unwrap();

    let src = format!(
        r#"
needs std.fs
let f = fs.open("{}", "r")
fs.read_bytes(f, 999999999999)
"#,
        test_file.display().to_string().replace('\\', "/")
    );

    let path = dir.path().join("huge_buffer.aelys");
    std::fs::write(&path, &src).unwrap();

    let mut config = aelys_runtime::VmConfig::default();
    config.capabilities.allow_fs = true;

    let result = aelys_driver::run_file_with_config(&path, config, Vec::new());
    match result {
        Err(AelysError::Runtime(runtime)) => {
            let msg = format!("{:?}", runtime.kind);
            assert!(
                msg.contains("buffer")
                    || msg.contains("maximum")
                    || msg.contains("size")
                    || msg.contains("max"),
                "expected buffer size error, got: {}",
                msg
            );
        }
        Err(e) => panic!("expected runtime error for huge buffer, got: {:?}", e),
        Ok(_) => panic!("expected error for huge buffer allocation"),
    }
}

/*
   verifies that ForLoopI/WhileLoopLt correctly validate consecutive register ranges
   previously, `a + 1` and `a + 2` were computed without overflow protection
*/
#[test]
fn register_index_overflow_is_handled() {
    let mut vm = make_vm();

    // ForLoopI uses 3 consecutive registers: a, a+1, a+2
    // with num_registers=5 and a=3, registers 3,4,5 would be needed but only 0-4 exist
    let mut func = Function::new(Some("forloop_oob".to_string()), 0);
    // ForLoopI with a=3 needs r3, r4, r5, but we'll limit to 5 registers (0-4)
    // emit_b format: (opcode, reg_a, imm16, line)
    func.emit_b(OpCode::ForLoopI, 3, -1, 1); // jump offset doesn't matter for this test
    func.emit_a(OpCode::Return0, 0, 0, 0, 1);

    func.finalize_bytecode();
    // override num_registers after finalize to force the oob condition
    func.num_registers = 5; // registers 0-4 available, but ForLoopI needs 3,4,5
    let func_ref = vm.alloc_function(func).unwrap();
    let err = vm.execute(func_ref).unwrap_err();
    match err.kind {
        RuntimeErrorKind::InvalidBytecode(msg) => {
            assert!(
                msg.contains("register") || msg.contains("ForLoopI"),
                "expected register bounds error, got: {}",
                msg
            );
        }
        _ => panic!(
            "expected InvalidBytecode for ForLoopI register OOB, got {:?}",
            err.kind
        ),
    }
}

#[test]
fn bytecode_rejects_invalid_nested_func_idx() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"VBXQ"); // Magic
    bytes.extend_from_slice(&1u16.to_le_bytes()); // Version major
    bytes.extend_from_slice(&0u16.to_le_bytes()); // Version minor
    bytes.extend_from_slice(&1u32.to_le_bytes()); // Flags
    bytes.extend_from_slice(&0u32.to_le_bytes()); // Entry point

    // Function 0:
    bytes.extend_from_slice(&0u16.to_le_bytes()); // Name length
    bytes.push(0u8); // Arity
    bytes.push(1u8); // Num registers
    bytes.extend_from_slice(&1u16.to_le_bytes()); // 1 constant

    // Constant: TAG_FUNC with invalid index
    bytes.push(5u8); // TAG_FUNC
    bytes.extend_from_slice(&999u32.to_le_bytes()); // func_idx = 999 (but 0 nested functions)

    bytes.extend_from_slice(&1u32.to_le_bytes()); // Bytecode length
    bytes.extend_from_slice(&0u32.to_le_bytes()); // Return0 opcode

    bytes.extend_from_slice(&0u16.to_le_bytes()); // Upvalue descriptors
    bytes.extend_from_slice(&0u16.to_le_bytes()); // Line numbers
    bytes.extend_from_slice(&0u32.to_le_bytes()); // Nested functions = 0
    bytes.extend_from_slice(&0u16.to_le_bytes()); // Global layout

    match deserialize(&bytes) {
        Err(BinaryError::InvalidNestedFunctionIndex { index, max }) => {
            assert_eq!(index, 999);
            assert!(max < 999);
        }
        Err(_) => {}
        Ok(_) => panic!("expected failure for invalid nested func_idx"),
    }
}

#[test]
fn gc_iterative_marking_handles_deep_closures() {
    let src = r#"
fn make_chain(n) {
    if n <= 0 {
        return fn() { return 0 }
    }
    let inner = make_chain(n - 1)
    return fn() { return inner() + 1 }
}
let chain = make_chain(50)
chain()
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("deep_closures.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(50));
}

#[test]
fn call_site_cache_slot_limit_enforced() {
    use aelys_runtime::MAX_CALL_SITE_SLOTS;
    const { assert!(MAX_CALL_SITE_SLOTS > 0) };
    const { assert!(MAX_CALL_SITE_SLOTS <= 65535) };
}

#[test]
fn integer_48bit_bounds_checked() {
    use aelys_runtime::Value;
    let max_val = Value::int(Value::INT_MAX);
    assert_eq!(max_val.as_int(), Some(Value::INT_MAX));
    let min_val = Value::int(Value::INT_MIN);
    assert_eq!(min_val.as_int(), Some(Value::INT_MIN));
    assert!(Value::int_checked(Value::INT_MAX + 1).is_err());
    assert!(Value::int_checked(Value::INT_MIN - 1).is_err());
}

#[test]
fn module_import_non_existent_symbol_fails() {
    let dir = tempfile::tempdir().unwrap();
    let mod_path = dir.path().join("mymod.aelys");
    std::fs::write(&mod_path, "pub let x = 1").unwrap();
    let main_path = dir.path().join("main.aelys");
    std::fs::write(&main_path, "needs nonexistent from mymod\nnonexistent").unwrap();
    let err = run_file(&main_path).unwrap_err();
    match err {
        AelysError::Compile(e) => {
            let msg = format!("{:?}", e.kind);
            assert!(
                msg.contains("not found") || msg.contains("Symbol") || msg.contains("nonexistent"),
                "expected symbol not found error, got: {}",
                msg
            );
        }
        _ => panic!("expected compile error for non-existent symbol import"),
    }
}

#[test]
fn closure_captures_preserve_values() {
    let src = r#"
fn make_adder(n) {
    return fn(x) { return n + x }
}
let add5 = make_adder(5)
let add10 = make_adder(10)
add5(3) + add10(7)
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("closures.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(25));
}

#[test]
fn deeply_nested_functions_work() {
    let src = r#"
fn l1(a) {
    fn l2(b) {
        fn l3(c) {
            fn l4(d) {
                fn l5(e) {
                    return a + b + c + d + e
                }
                return l5
            }
            return l4
        }
        return l3
    }
    return l2
}
l1(1)(2)(3)(4)(5)
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("nested.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn manual_heap_operations_validated() {
    let src = r#"
        let h = alloc(5)
        store(h, 0, 100)
        store(h, 4, 400)
        let a = load(h, 0)
        let b = load(h, 4)
        free(h)
        a + b
    "#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("heap.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(500));
}

#[test]
fn recursive_function_handles_deep_calls() {
    let src = r#"
fn sum_to(n) {
    if n <= 0 {
        return 0
    }
    return n + sum_to(n - 1)
}
sum_to(100)
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("recursive.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(5050));
}

#[test]
fn inline_cache_works_correctly() {
    let src = r#"
fn double(x) {
    return x * 2
}
let mut sum = 0
let mut i = 0
while i < 1000 {
    sum += double(i)
    i++
}
sum
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cache.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    let expected: i64 = (0..1000).map(|x| x * 2).sum();
    assert_eq!(result.as_int(), Some(expected));
}

#[test]
fn compiler_rc_shared_state() {
    use aelys_backend::Compiler;
    use aelys_syntax::Source;
    use std::rc::Rc;
    let source = Source::new("<test>", "let x = 1");
    let compiler = Compiler::new(None, source);
    let count1 = Rc::strong_count(&compiler.module_aliases);
    let count2 = Rc::strong_count(&compiler.known_globals);
    let count3 = Rc::strong_count(&compiler.known_native_globals);
    assert!(count1 >= 1);
    assert!(count2 >= 1);
    assert!(count3 >= 1);
    let _alias_clone = Rc::clone(&compiler.module_aliases);
    assert!(Rc::strong_count(&compiler.module_aliases) >= 2);
}

#[test]
fn type_inference_rc_shared() {
    use aelys_sema::{InferType, TypeEnv};
    use std::rc::Rc;
    let mut env = TypeEnv::new();
    let fn_type = Rc::new(InferType::Function {
        params: vec![InferType::I64],
        ret: Box::new(InferType::I64),
    });
    env.define_function("test_fn".to_string(), Rc::clone(&fn_type));
    assert_eq!(Rc::strong_count(&fn_type), 2);
    let looked_up = env.lookup_function("test_fn");
    assert!(looked_up.is_some());
}

#[test]
fn value_nan_boxing_preserves_types() {
    use aelys_runtime::Value;
    let int_val = Value::int(42);
    assert!(int_val.is_int());
    assert!(!int_val.is_float());
    assert!(!int_val.is_bool());
    assert!(!int_val.is_null());
    let float_val = Value::float(2.72);
    assert!(float_val.is_float());
    assert!(!float_val.is_int());
    let bool_val = Value::bool(true);
    assert!(bool_val.is_bool());
    let null_val = Value::null();
    assert!(null_val.is_null());
    let ptr_val = Value::ptr(12345);
    assert!(ptr_val.is_ptr());
}

#[test]
fn arithmetic_operations_work() {
    let src = r#"
        let a = 10 + 5
        let b = 10 - 5
        let c = 10 * 5
        let d = 10 / 5
        let e = 10 % 3
        a + b + c + d + e
    "#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("arith.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(15 + 5 + 50 + 2 + 1));
}

#[test]
fn comparison_operations_work() {
    let src = r#"
let a = 5 < 10
let b = 10 > 5
let c = 5 <= 5
let d = 5 >= 5
let e = 5 == 5
let f = 5 != 6
let result = if a and b and c and d and e and f { 1 } else { 0 }
result
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("cmp.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn logical_short_circuit_and() {
    let src = r#"
let mut called = false
fn side_effect() {
    called = true
    return true
}
let result = false and side_effect()
let out = if called { 1 } else { 0 }
out
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("short_and.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(0));
}

#[test]
fn logical_short_circuit_or() {
    let src = r#"
let mut called = false
fn side_effect() {
    called = true
    return true
}
let result = true or side_effect()
let out = if called { 1 } else { 0 }
out
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("short_or.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(0));
}

#[test]
fn while_loop_with_break() {
    let src = r#"
let mut i = 0
while true {
    i++
    if i >= 50 {
        break
    }
}
i
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("break.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(50));
}

#[test]
fn while_loop_with_continue() {
    let src = r#"
let mut i = 0
let mut sum = 0
while i < 20 {
    i++
    if i % 2 == 0 {
        continue
    }
    sum += i
}
sum
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("continue.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    let expected: i64 = (1..=20).filter(|x| x % 2 != 0).sum();
    assert_eq!(result.as_int(), Some(expected));
}

#[test]
fn string_concatenation() {
    let src = r#"
let s = "hello" + " " + "world"
s.len()
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("strings.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(11));
}

#[test]
fn mutual_recursion_works() {
    let src = r#"
fn is_even(n) {
    if n == 0 {
        return true
    }
    return is_odd(n - 1)
}
fn is_odd(n) {
    if n == 0 {
        return false
    }
    return is_even(n - 1)
}
let result = if is_even(50) and is_odd(51) { 1 } else { 0 }
result
"#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("mutual.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn variable_shadowing() {
    let src = r#"
        let x = 1
        {
            let x = 2
            {
                let x = 3
            }
        }
        x
    "#;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("shadow.aelys");
    std::fs::write(&path, src).unwrap();
    let result = run_file(&path).unwrap();
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn globals_sync_includes_null_values() {
    /* verifies that setting a global to null via one module is visible in another
    previously, null values were skipped during sync causing inconsistencies. */
    let dir = tempfile::tempdir().unwrap();

    let helper = dir.path().join("helper.aelys");
    std::fs::write(
        &helper,
        r#"
pub let mut shared = 42

pub fn set_to_null() {
    shared = null
}

pub fn get_shared() {
    return shared
}
"#,
    )
    .unwrap();

    let main = dir.path().join("main.aelys");
    std::fs::write(
        &main,
        r#"
needs shared, set_to_null, get_shared from helper

let before = get_shared()
set_to_null()
let after = get_shared()

if before == 42 and after == null { 1 } else { 0 }
"#,
    )
    .unwrap();

    let result = run_file(&main).unwrap();
    assert_eq!(result.as_int(), Some(1));
}

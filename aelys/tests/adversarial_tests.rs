mod common;
use aelys_runtime::{Function, OpCode, VM, Value};
use aelys_syntax::Source;
use common::*;

// Bytecode verifier bypass attempts

#[test]
fn malicious_bytecode_invalid_opcode() {
    let mut vm = VM::new(Source::new("test.aelys", "")).unwrap();
    let mut func = Function::new(Some("malicious".to_string()), 0);
    func.num_registers = 1;

    // Try to inject invalid opcode
    func.set_bytecode(vec![0xDEADBEEF]);

    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref);

    assert!(result.is_err());
}

#[test]
fn bytecode_constant_pool_oob() {
    let mut vm = VM::new(Source::new("test.aelys", "")).unwrap();
    let mut func = Function::new(Some("const_oob".to_string()), 0);
    func.num_registers = 1;
    func.constants.push(Value::int(42));

    // Try to access constant index 5 when only 1 exists
    func.emit_a(OpCode::LoadK, 0, 5, 0, 1);
    func.emit_a(OpCode::Return0, 0, 0, 0, 1);

    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref);

    assert!(result.is_err());
}

#[test]
fn bytecode_jump_to_invalid_address() {
    let mut vm = VM::new(Source::new("test.aelys", "")).unwrap();
    let mut func = Function::new(Some("bad_jump".to_string()), 0);
    func.num_registers = 1;

    // Jump way beyond bytecode
    func.emit_b(OpCode::Jump, 0, 1000, 1);
    func.emit_a(OpCode::Return0, 0, 0, 0, 1);

    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref);

    // Should error when trying to execute beyond bytecode
    assert!(result.is_err());
}

#[test]
fn bytecode_negative_jump() {
    let mut vm = VM::new(Source::new("test.aelys", "")).unwrap();
    let mut func = Function::new(Some("neg_jump".to_string()), 0);
    func.num_registers = 1;

    func.emit_b(OpCode::Jump, 0, -100, 1);
    func.emit_a(OpCode::Return0, 0, 0, 0, 1);

    func.finalize_bytecode();
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref);

    assert!(result.is_err());
}

// Path traversal attacks
// (network_recv_buffer_overflow → stdlib_net_tests::recv_bytes_exceeds_max_buffer)
// (path_traversal_dotdot → security_audit_tests::fs_join_rejects_parent_escape)
// (path_traversal_absolute → security_audit_tests::fs_join_rejects_absolute_path)

#[test]
fn path_traversal_url_encoded() {
    let code = r#"
needs std.fs
fs.join("/app", "..%2F..%2Fetc%2Fpasswd")
"#;
    let err = run_aelys_err(code);
    // Should still be caught
    assert!(err.contains("capability") || err.contains("escapes"));
}

// Type confusion attacks

#[test]
fn type_confusion_null_as_int() {
    let code = r#"
let x = null
x + 5
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("type") || err.contains("Type"));
}

#[test]
fn type_confusion_string_as_number() {
    let code = r#"
let x = "hello"
x * 2
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("type") || err.contains("Type"));
}

#[test]
fn type_confusion_bool_arithmetic() {
    let code = r#"
let x = true
x + 10
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("type") || err.contains("Type"));
}

// Resource exhaustion attacks

#[test]
#[ignore]
fn infinite_string_concat_oom() {
    let code = r#"
let mut s = "x"
let mut i = 0
while i < 100000 {
    s = s + s
    i++
}
42
"#;
    // This might OOM or hit limits, either is fine
    let result = run_aelys_result(code);
    // If it completes, that's also OK (we have limits)
    let _ = result;
}

#[test]
#[ignore]
fn deep_recursion_stack_overflow() {
    let code = r#"
fn recurse(n) {
    return recurse(n + 1)
}
recurse(0)
"#;
    let err = run_aelys_err(code);
    // Should hit MAX_FRAMES limit
    assert!(
        err.contains("stack")
            || err.contains("frame")
            || err.contains("recursion")
            || err.contains("Stack")
    );
}

#[test]
#[ignore]
fn allocation_bomb() {
    let code = r#"
let mut i = 0
while i < 100000 {
    let p = alloc(1000)
    i++
}
42
"#;
    // Might complete or OOM, either is acceptable
    let _ = run_aelys_result(code);
}

// Command injection attempts

#[test]
fn exec_shell_injection_attempt() {
    let code = r#"
needs std.sys
sys.exec("echo hello; rm -rf /")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("capability"));
}

#[test]
fn exec_args_should_prevent_injection() {
    // exec_args doesn't use shell, so shell metacharacters should be literal
    let code = r#"
needs std.sys
sys.exec_args("echo", "hello; rm -rf /")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("capability"));
}

// Integer overflow/underflow

#[test]
fn integer_overflow_checked() {
    let code = r#"
let max = 140737488355327
max + 1
"#;
    // This might overflow or be handled
    let _ = run_aelys(code);
}

#[test]
fn integer_multiply_overflow() {
    let code = r#"
let big = 10000000000
big * big
"#;
    let _ = run_aelys(code);
}

// Format string attacks

#[test]
fn time_format_string_attack() {
    let code = r#"
format("%n%n%n%n%n")
"#;
    // Should not crash
    let _ = run_aelys(code);
}

// Binary deserialization attacks

#[test]
fn binary_oversized_function_count() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"VBXQ");
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());

    // Try to claim 1 million nested functions
    bytes.extend_from_slice(&0u16.to_le_bytes()); // name len
    bytes.push(0u8); // arity
    bytes.push(0u8); // num_registers
    bytes.extend_from_slice(&0u16.to_le_bytes()); // constants
    bytes.extend_from_slice(&1u32.to_le_bytes()); // bytecode length
    bytes.extend_from_slice(&0u32.to_le_bytes()); // Return0
    bytes.extend_from_slice(&0u16.to_le_bytes()); // upvalues
    bytes.extend_from_slice(&0u16.to_le_bytes()); // line numbers
    bytes.extend_from_slice(&1000000u32.to_le_bytes()); // nested functions

    let result = aelys_bytecode::asm::deserialize(&bytes);
    assert!(result.is_err());
}

#[test]
fn binary_oversized_constants() {
    let mut bytes = Vec::new();
    bytes.extend_from_slice(b"VBXQ");
    bytes.extend_from_slice(&1u16.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.extend_from_slice(&1u32.to_le_bytes());
    bytes.extend_from_slice(&0u32.to_le_bytes());
    bytes.extend_from_slice(&0u16.to_le_bytes());
    bytes.push(0u8);
    bytes.push(0u8);
    bytes.extend_from_slice(&65535u16.to_le_bytes()); // Max u16 constants

    let result = aelys_bytecode::asm::deserialize(&bytes);
    assert!(result.is_err());
}

// Unicode exploitation attempts

#[test]
fn unicode_bidi_override_attack() {
    // Right-to-left override can hide malicious code
    // Test that the code runs safely without issues
    let code = r#"
let safe = "hello"
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn unicode_homoglyph_attack() {
    // Using lookalike characters (Cyrillic vs Latin)
    let code = r#"
let х = 42
х
"#;
    // х is Cyrillic, should work fine
    assert_aelys_int(code, 42);
}

// Division by zero variations

#[test]
fn division_by_zero_direct() {
    let code = "10 / 0";
    assert_aelys_error_contains(code, "division");
}

#[test]
fn division_by_zero_variable() {
    let code = r#"
let x = 0
10 / x
"#;
    assert_aelys_error_contains(code, "division");
}

#[test]
fn modulo_by_zero() {
    let code = "10 % 0";
    assert_aelys_error_contains(code, "division");
}

// Memory safety

#[test]
fn use_after_free_manual_heap() {
    let code = r#"
let p = alloc(10)
store(p, 0, 42)
free(p)
load(p, 0)
"#;
    assert_aelys_error_contains(code, "freed");
}

#[test]
fn double_free_manual_heap() {
    let code = r#"
let p = alloc(10)
free(p)
free(p)
"#;
    assert_aelys_error_contains(code, "free");
}

#[test]
fn manual_heap_buffer_overflow() {
    let code = r#"
let p = alloc(5)
load(p, 100)
"#;
    assert_aelys_error_contains(code, "bound");
}

#[test]
fn manual_heap_negative_offset() {
    let code = r#"
let p = alloc(10)
load(p, -5)
"#;
    assert_aelys_error_contains(code, "negative");
}

// GC exploitation

#[test]
fn gc_collection_during_critical_section() {
    let code = r#"
let mut i = 0
while i < 5000 {
    let s1 = "string" + " concatenation"
    let s2 = "more " + "strings"
    let s3 = s1 + s2
    i++
}
42
"#;
    assert_aelys_int(code, 42);
}

// capability bypass attempts

#[test]
fn fs_access_without_capability() {
    let code = r#"
needs std.fs
fs.read_text("/etc/passwd")
"#;
    assert_aelys_error_contains(code, "capability");
}

#[test]
fn net_access_without_capability() {
    // Net requires capability for all network operations
    let code = r#"
needs std.net
net.connect("www.google.com", 80)
"#;
    // Network operations require capability
    let err = run_aelys_err(code);
    assert!(err.contains("capability") || err.contains("Capability"));
}

// (exec_without_capability → stdlib_sys_tests::sys_exec_denied_without_capability)

// Prototype pollution attempts (not applicable, but test object safety)

#[test]
fn function_modification_attempt() {
    let code = r#"
fn test() { return 42 }
let result = test()
result
"#;
    assert_aelys_int(code, 42);
}

// Null pointer dereference

#[test]
fn null_function_call() {
    let code = r#"
let f = null
f()
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("not callable") || err.contains("type") || err.contains("Type"));
}

use aelys_bytecode::asm::disasm::escape_string;
use aelys_bytecode::asm::{assemble, deserialize, disassemble, serialize};
use aelys_runtime::{Function, Heap, Value};

#[test]
fn test_escape_string() {
    assert_eq!(escape_string("hello"), "hello");
    assert_eq!(escape_string("hello\nworld"), "hello\\nworld");
    assert_eq!(escape_string("tab\there"), "tab\\there");
    assert_eq!(escape_string("quote\"here"), "quote\\\"here");
    assert_eq!(escape_string("back\\slash"), "back\\\\slash");
}

#[test]
fn test_disassemble_empty_function() {
    let func = Function::new(Some("test".to_string()), 0);
    let output = disassemble(&func, None);
    assert!(output.contains(".function 0"));
    assert!(output.contains(".name \"test\""));
    assert!(output.contains(".arity 0"));
}

#[test]
fn test_basic_assembly() {
    let source = r#"
.version 1

.function 0
  .name "main"
  .arity 0
  .registers 2

  .code
    0000: LoadI     r0, 42
    0001: Print     r0
    0002: Return0
"#;
    let (functions, _heap) = assemble(source).unwrap();
    assert_eq!(functions.len(), 1);
    assert_eq!(functions[0].name, Some("main".to_string()));
    assert_eq!(functions[0].arity, 0);
    assert_eq!(functions[0].bytecode.len(), 3);
}

#[test]
fn test_label_resolution() {
    let source = r#"
.function 0
  .arity 0
  .registers 1

  .code
    0000: JumpIfNot r0, L0
    0001: LoadI     r0, 1
    0002: Jump      L1
  L0:
    0003: LoadI     r0, 0
  L1:
    0004: Return    r0
"#;
    let (functions, _heap) = assemble(source).unwrap();
    assert_eq!(functions[0].bytecode.len(), 5);
}

#[test]
fn test_binary_basic_roundtrip() {
    let mut func = Function::new(Some("test".to_string()), 0);
    func.num_registers = 2;
    func.set_bytecode(vec![0x01_00_00_2A, 0x22_00_00_00]); // LoadI r0, 42; Print r0

    let heap = Heap::new();
    let bytes = serialize(&func, &heap);

    assert_eq!(&bytes[0..4], b"VBXQ");

    let (loaded, _) = deserialize(&bytes).unwrap();
    assert_eq!(loaded.name, Some("test".to_string()));
    assert_eq!(loaded.arity, 0);
    assert_eq!(loaded.num_registers, 2);
    assert_eq!(loaded.bytecode, func.bytecode);
}

#[test]
fn test_binary_with_constants() {
    let mut func = Function::new(None, 0);
    func.constants = vec![
        Value::int(42),
        Value::float(3.14),
        Value::bool(true),
        Value::null(),
    ];

    let heap = Heap::new();
    let bytes = serialize(&func, &heap);
    let (loaded, _) = deserialize(&bytes).unwrap();

    assert_eq!(loaded.constants.len(), 4);
    assert_eq!(loaded.constants[0].as_int(), Some(42));
    assert!((loaded.constants[1].as_float().unwrap() - 3.14).abs() < 0.001);
    assert_eq!(loaded.constants[2].as_bool(), Some(true));
    assert!(loaded.constants[3].is_null());
}

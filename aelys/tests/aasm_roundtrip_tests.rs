use aelys_backend::Compiler;
use aelys_bytecode::asm::{assemble, deserialize, disassemble, serialize};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_runtime::Heap;
use aelys_runtime::{VM, Value};
use aelys_syntax::Source;

/// Run source code directly
fn run_source(source: &str) -> Value {
    aelys::run(source, "<test>").expect("Execution failed")
}

/// Helper to compile source to bytecode and heap
fn compile_source(source: &str) -> (aelys_runtime::Function, Heap) {
    let src = Source::new("<test>", source);
    let tokens = Lexer::with_source(src.clone())
        .scan()
        .expect("Lexer failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("Parser failed");

    // Use typed compilation pipeline
    let typed_program = aelys_sema::TypeInference::infer_program(stmts, src.clone())
        .expect("Type inference failed");
    let (func, heap, _globals) = Compiler::new(None, src)
        .compile_typed(&typed_program)
        .expect("Compiler failed");

    (func, heap)
}

/// Run a function with a fresh heap (for roundtrip testing)
fn run_function_with_heap(mut func: aelys_runtime::Function, mut heap: Heap) -> Value {
    let src = Source::new("<test>", "");
    let mut vm = VM::new(src).unwrap();

    let remap = vm.merge_heap(&mut heap).unwrap();
    func.remap_constants(&remap);

    let func_ref = vm.alloc_function(func).unwrap();
    vm.execute(func_ref).expect("Execution failed")
}

#[test]
fn test_asm_roundtrip_simple() {
    let source = "42";
    let result_direct = run_source(source);

    // Roundtrip through .aasm
    let (func, heap) = compile_source(source);
    let asm_text = disassemble(&func, Some(&heap));
    let (functions, asm_heap) = assemble(&asm_text).expect("Assemble failed");
    let result_roundtrip = run_function_with_heap(functions.into_iter().next().unwrap(), asm_heap);

    assert_eq!(result_direct.as_int(), result_roundtrip.as_int());
}

#[test]
fn test_bytecode_asm_roundtrip_api() {
    let src = r#"
.function 0
  .name "main"
  .arity 0
  .registers 0

  .code
    0000: Return0
"#;
    let (functions, heap) = aelys_bytecode::asm::assemble(src).expect("Assemble failed");
    let text = aelys_bytecode::asm::disassemble(&functions[0], Some(&heap));
    assert!(text.contains(".function 0"));
}

#[test]
fn test_asm_roundtrip_arithmetic() {
    let source = "(10 + 5) * 2 - 3";
    let result_direct = run_source(source);

    let (func, heap) = compile_source(source);
    let asm_text = disassemble(&func, Some(&heap));
    let (functions, asm_heap) = assemble(&asm_text).expect("Assemble failed");
    let result_roundtrip = run_function_with_heap(functions.into_iter().next().unwrap(), asm_heap);

    assert_eq!(result_direct.as_int(), result_roundtrip.as_int());
    assert_eq!(result_direct.as_int(), Some(27));
}

#[test]
fn test_asm_roundtrip_conditionals() {
    // Simple conditional expression (not if-else statement)
    // Use ternary-style: the last expression is returned
    let source = "10 > 5"; // Returns true/false - simpler test for jumps
    let result_direct = run_source(source);

    let (func, heap) = compile_source(source);
    let asm_text = disassemble(&func, Some(&heap));
    let (functions, asm_heap) = assemble(&asm_text).expect("Assemble failed");
    let result_roundtrip = run_function_with_heap(functions.into_iter().next().unwrap(), asm_heap);

    assert_eq!(result_direct.as_bool(), result_roundtrip.as_bool());
    assert_eq!(result_direct.as_bool(), Some(true));
}

#[test]
fn test_asm_roundtrip_while_loop() {
    let source = r#"
        let mut sum = 0
        let mut i = 1
        while i <= 5 {
            sum += i
            i++
        }
        sum
    "#;
    let result_direct = run_source(source);

    let (func, heap) = compile_source(source);
    let asm_text = disassemble(&func, Some(&heap));
    let (functions, asm_heap) = assemble(&asm_text).expect("Assemble failed");
    let result_roundtrip = run_function_with_heap(functions.into_iter().next().unwrap(), asm_heap);

    assert_eq!(result_direct.as_int(), result_roundtrip.as_int());
    assert_eq!(result_direct.as_int(), Some(15)); // 1+2+3+4+5
}

#[test]
fn test_binary_roundtrip_simple() {
    let source = "42";
    let result_direct = run_source(source);

    // Roundtrip through .avbc
    let (func, heap) = compile_source(source);
    let bytes = serialize(&func, &heap);
    let (loaded_func, loaded_heap) = deserialize(&bytes).expect("Deserialize failed");
    let result_roundtrip = run_function_with_heap(loaded_func, loaded_heap);

    assert_eq!(result_direct.as_int(), result_roundtrip.as_int());
}

#[test]
fn test_binary_roundtrip_with_strings() {
    // Note: We can't easily capture print output, so we just verify the roundtrip works
    let source = r#"
        let x = "hello"
        42
    "#;
    let result_direct = run_source(source);

    let (func, heap) = compile_source(source);
    let bytes = serialize(&func, &heap);
    let (loaded_func, loaded_heap) = deserialize(&bytes).expect("Deserialize failed");
    let result_roundtrip = run_function_with_heap(loaded_func, loaded_heap);

    assert_eq!(result_direct.as_int(), result_roundtrip.as_int());
}

#[test]
fn test_binary_roundtrip_with_floats() {
    let source = "3.14159";
    let result_direct = run_source(source);

    let (func, heap) = compile_source(source);
    let bytes = serialize(&func, &heap);
    let (loaded_func, loaded_heap) = deserialize(&bytes).expect("Deserialize failed");
    let result_roundtrip = run_function_with_heap(loaded_func, loaded_heap);

    let direct_f = result_direct.as_float().expect("Expected float");
    let roundtrip_f = result_roundtrip.as_float().expect("Expected float");
    assert!((direct_f - roundtrip_f).abs() < 0.00001);
}

#[test]
fn test_binary_roundtrip_conditionals() {
    let source = r#"
        let x = 10
        if x > 5 { x + 1 } else { x - 1 }
    "#;
    let result_direct = run_source(source);

    let (func, heap) = compile_source(source);
    let bytes = serialize(&func, &heap);
    let (loaded_func, loaded_heap) = deserialize(&bytes).expect("Deserialize failed");
    let result_roundtrip = run_function_with_heap(loaded_func, loaded_heap);

    assert_eq!(result_direct.as_int(), result_roundtrip.as_int());
}

#[test]
fn test_double_roundtrip() {
    // source -> bytecode -> .aasm -> bytecode -> .avbc -> bytecode
    let source = r#"
        let x = 10
        let y = 20
        x + y
    "#;
    let result_direct = run_source(source);

    // First roundtrip: through .aasm
    let (func, heap) = compile_source(source);
    let asm_text = disassemble(&func, Some(&heap));
    let (asm_funcs, asm_heap) = assemble(&asm_text).expect("Assemble failed");

    // Second roundtrip: through .avbc
    let bytes = serialize(&asm_funcs[0], &asm_heap);
    let (final_func, final_heap) = deserialize(&bytes).expect("Deserialize failed");
    let result_final = run_function_with_heap(final_func, final_heap);

    assert_eq!(result_direct.as_int(), result_final.as_int());
    assert_eq!(result_direct.as_int(), Some(30));
}

#[test]
fn test_empty_function() {
    let func = aelys_runtime::Function::new(Some("empty".to_string()), 0);
    let heap = Heap::new();

    // Test disassemble
    let asm_text = disassemble(&func, Some(&heap));
    assert!(asm_text.contains(".function 0"));
    assert!(asm_text.contains(".name \"empty\""));

    // Test binary roundtrip
    let bytes = serialize(&func, &heap);
    let (loaded, _) = deserialize(&bytes).expect("Deserialize failed");
    assert_eq!(loaded.name, Some("empty".to_string()));
}

#[test]
fn test_negative_numbers() {
    let source = "-42";
    let result_direct = run_source(source);

    let (func, heap) = compile_source(source);
    let asm_text = disassemble(&func, Some(&heap));
    let (functions, asm_heap) = assemble(&asm_text).expect("Assemble failed");
    let result_roundtrip = run_function_with_heap(functions.into_iter().next().unwrap(), asm_heap);

    assert_eq!(result_direct.as_int(), result_roundtrip.as_int());
    assert_eq!(result_direct.as_int(), Some(-42));
}

#[test]
fn test_large_numbers() {
    let source = "123456789";
    let result_direct = run_source(source);

    // Binary roundtrip
    let (func, heap) = compile_source(source);
    let bytes = serialize(&func, &heap);
    let (loaded_func, loaded_heap) = deserialize(&bytes).expect("Deserialize failed");
    let result_roundtrip = run_function_with_heap(loaded_func, loaded_heap);

    assert_eq!(result_direct.as_int(), result_roundtrip.as_int());
}

#[test]
fn test_booleans() {
    let source = "true";
    let result_direct = run_source(source);

    let (func, heap) = compile_source(source);
    let bytes = serialize(&func, &heap);
    let (loaded_func, loaded_heap) = deserialize(&bytes).expect("Deserialize failed");
    let result_roundtrip = run_function_with_heap(loaded_func, loaded_heap);

    assert_eq!(result_direct.as_bool(), result_roundtrip.as_bool());
    assert_eq!(result_direct.as_bool(), Some(true));
}

#[test]
fn test_null() {
    let source = "null";
    let result_direct = run_source(source);

    let (func, heap) = compile_source(source);
    let bytes = serialize(&func, &heap);
    let (loaded_func, loaded_heap) = deserialize(&bytes).expect("Deserialize failed");
    let result_roundtrip = run_function_with_heap(loaded_func, loaded_heap);

    assert!(result_direct.is_null());
    assert!(result_roundtrip.is_null());
}

#[test]
fn test_string_escaping() {
    // Test that strings with special characters survive roundtrip
    let source = r#""hello\nworld""#;
    let (func, heap) = compile_source(source);

    // Just verify it doesn't crash
    let asm_text = disassemble(&func, Some(&heap));
    assert!(asm_text.contains("\\n")); // Should be escaped

    let bytes = serialize(&func, &heap);
    let (_, _) = deserialize(&bytes).expect("Deserialize failed");
}

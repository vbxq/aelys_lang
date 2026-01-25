use aelys_backend::Compiler;
use aelys_frontend::{lexer::Lexer, parser::Parser};
use aelys_runtime::{VM, VmConfig};
use aelys_sema::TypeInference;
use aelys_syntax::Source;

#[test]
fn vm_executes_bytecode() {
    let src = Source::new("<t>", "fn f() -> int { 1 } f()");
    let tokens = Lexer::with_source(src.clone()).scan().unwrap();
    let ast = Parser::new(tokens, src.clone()).parse().unwrap();
    let typed = TypeInference::infer_program(ast, src.clone()).unwrap();
    let (mut func, mut heap, _) = Compiler::new(None, src.clone())
        .compile_typed(&typed)
        .unwrap();

    let mut vm = VM::with_config_and_args(src, VmConfig::default(), vec![]).unwrap();
    let remap = vm.merge_heap(&mut heap).unwrap();
    func.remap_constants(&remap);
    let func_ref = vm.alloc_function(func).unwrap();
    let result = vm.execute(func_ref).unwrap();

    assert_eq!(result.to_string(), "1");
}

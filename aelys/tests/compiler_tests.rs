use aelys_backend::Compiler;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_syntax::Source;

#[test]
fn compile_typed_program_to_bytecode() {
    let src = Source::new("<t>", "fn f() -> string { \"hi\" } f()");
    let tokens = Lexer::with_source(src.clone()).scan().unwrap();
    let ast = Parser::new(tokens, src.clone()).parse().unwrap();
    let typed = TypeInference::infer_program(ast, src.clone()).unwrap();

    let (func, heap, _globals) = Compiler::new(None, src).compile_typed(&typed).unwrap();
    assert!(!func.bytecode.is_empty());
    assert!(heap.object_count() > 0);
}

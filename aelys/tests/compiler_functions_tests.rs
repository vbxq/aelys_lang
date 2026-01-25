use aelys_backend::compiler::Compiler;
use aelys_syntax::ast::{Expr, ExprKind, Function, Stmt, StmtKind};
use aelys_syntax::{Source, Span};

#[test]
fn untyped_function_merges_call_site_slots_without_implicit_return() {
    let span = Span::dummy();
    let source = Source::new("<test>", "");
    let mut compiler = Compiler::new(None, source);
    compiler.globals.insert("callee".to_string(), false);

    let call_expr = Expr::new(
        ExprKind::Call {
            callee: Box::new(Expr::new(ExprKind::Identifier("callee".to_string()), span)),
            args: Vec::new(),
        },
        span,
    );

    let func = Function {
        name: "f".to_string(),
        params: Vec::new(),
        return_type: None,
        body: vec![
            Stmt::new(StmtKind::Expression(call_expr), span),
            Stmt::new(StmtKind::Return(None), span),
        ],
        decorators: Vec::new(),
        is_pub: false,
        span,
    };

    compiler.compile_function(&func).unwrap();

    assert!(compiler.next_call_site_slot > 0);
}

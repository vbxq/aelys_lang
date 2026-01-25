use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_syntax::{BinaryOp, ExprKind, Source, StmtKind, UnaryOp};

fn parse(source: &str) -> Vec<aelys_syntax::Stmt> {
    let src = Source::new("<test>", source);
    let tokens = Lexer::with_source(src.clone()).scan().unwrap();
    Parser::new(tokens, src).parse().unwrap()
}

#[test]
fn test_parse_simple_function() {
    let stmts = parse("fn add(a: int, b: int) -> int { a + b }");
    assert!(!stmts.is_empty());
}

#[test]
fn test_let_statement() {
    let stmts = parse("let x = 42");
    assert_eq!(stmts.len(), 1);

    match &stmts[0].kind {
        StmtKind::Let {
            name,
            mutable,
            initializer,
            ..
        } => {
            assert_eq!(name, "x");
            assert!(!mutable);
            assert!(matches!(initializer.kind, ExprKind::Int(42)));
        }
        _ => panic!("Expected let statement"),
    }
}

#[test]
fn test_let_mut_statement() {
    let stmts = parse("let mut y = 10");

    match &stmts[0].kind {
        StmtKind::Let { name, mutable, .. } => {
            assert_eq!(name, "y");
            assert!(*mutable);
        }
        _ => panic!("Expected let statement"),
    }
}

#[test]
fn test_binary_expression() {
    let stmts = parse("1 + 2 * 3");

    match &stmts[0].kind {
        StmtKind::Expression(expr) => {
            // Should be (1 + (2 * 3)) due to precedence
            match &expr.kind {
                ExprKind::Binary { left, op, right } => {
                    assert!(matches!(left.kind, ExprKind::Int(1)));
                    assert_eq!(*op, BinaryOp::Add);
                    match &right.kind {
                        ExprKind::Binary { left, op, right } => {
                            assert!(matches!(left.kind, ExprKind::Int(2)));
                            assert_eq!(*op, BinaryOp::Mul);
                            assert!(matches!(right.kind, ExprKind::Int(3)));
                        }
                        _ => panic!("Expected nested binary"),
                    }
                }
                _ => panic!("Expected binary expression"),
            }
        }
        _ => panic!("Expected expression statement"),
    }
}

#[test]
fn test_unary_expression() {
    let stmts = parse("-42");

    match &stmts[0].kind {
        StmtKind::Expression(expr) => match &expr.kind {
            ExprKind::Unary { op, operand } => {
                assert_eq!(*op, UnaryOp::Neg);
                assert!(matches!(operand.kind, ExprKind::Int(42)));
            }
            _ => panic!("Expected unary expression"),
        },
        _ => panic!("Expected expression statement"),
    }
}

#[test]
fn test_function_call() {
    let stmts = parse("foo(1, 2, 3)");

    match &stmts[0].kind {
        StmtKind::Expression(expr) => match &expr.kind {
            ExprKind::Call { callee, args } => {
                assert!(matches!(&callee.kind, ExprKind::Identifier(n) if n == "foo"));
                assert_eq!(args.len(), 3);
            }
            _ => panic!("Expected call expression"),
        },
        _ => panic!("Expected expression statement"),
    }
}

#[test]
fn test_if_statement() {
    let stmts = parse("if x > 0 { y }");

    match &stmts[0].kind {
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            assert!(matches!(
                &condition.kind,
                ExprKind::Binary {
                    op: BinaryOp::Gt,
                    ..
                }
            ));
            assert!(matches!(&then_branch.kind, StmtKind::Block(_)));
            assert!(else_branch.is_none());
        }
        _ => panic!("Expected if statement"),
    }
}

#[test]
fn test_if_else_statement() {
    let stmts = parse("if a { b } else { c }");

    match &stmts[0].kind {
        StmtKind::If { else_branch, .. } => {
            assert!(else_branch.is_some());
        }
        _ => panic!("Expected if statement"),
    }
}

#[test]
fn test_while_statement() {
    let stmts = parse("while x < 10 { x = x + 1 }");

    match &stmts[0].kind {
        StmtKind::While { condition, body } => {
            assert!(matches!(
                &condition.kind,
                ExprKind::Binary {
                    op: BinaryOp::Lt,
                    ..
                }
            ));
            assert!(matches!(&body.kind, StmtKind::Block(_)));
        }
        _ => panic!("Expected while statement"),
    }
}

#[test]
fn test_function_declaration() {
    let stmts = parse("fn add(a, b) { a + b }");

    match &stmts[0].kind {
        StmtKind::Function(func) => {
            assert_eq!(func.name, "add");
            let param_names: Vec<&str> = func.params.iter().map(|p| p.name.as_str()).collect();
            assert_eq!(param_names, vec!["a", "b"]);
            assert_eq!(func.body.len(), 1);
        }
        _ => panic!("Expected function declaration"),
    }
}

#[test]
fn test_function_with_decorator() {
    let stmts = parse("@no_gc\nfn hot() { }");

    match &stmts[0].kind {
        StmtKind::Function(func) => {
            assert_eq!(func.name, "hot");
            assert_eq!(func.decorators.len(), 1);
            assert_eq!(func.decorators[0].name, "no_gc");
        }
        _ => panic!("Expected function declaration"),
    }
}

#[test]
fn test_and_or_expressions() {
    let stmts = parse("a and b or c");

    // Should be ((a and b) or c)
    match &stmts[0].kind {
        StmtKind::Expression(expr) => match &expr.kind {
            ExprKind::Or { left, right } => {
                assert!(matches!(&left.kind, ExprKind::And { .. }));
                assert!(matches!(&right.kind, ExprKind::Identifier(n) if n == "c"));
            }
            _ => panic!("Expected or expression"),
        },
        _ => panic!("Expected expression statement"),
    }
}

#[test]
fn test_assignment() {
    let stmts = parse("x = 42");

    match &stmts[0].kind {
        StmtKind::Expression(expr) => match &expr.kind {
            ExprKind::Assign { name, value } => {
                assert_eq!(name, "x");
                assert!(matches!(value.kind, ExprKind::Int(42)));
            }
            _ => panic!("Expected assignment"),
        },
        _ => panic!("Expected expression statement"),
    }
}

#[test]
fn test_return_with_value() {
    let stmts = parse("return 42");

    match &stmts[0].kind {
        StmtKind::Return(Some(expr)) => {
            assert!(matches!(expr.kind, ExprKind::Int(42)));
        }
        _ => panic!("Expected return with value"),
    }
}

#[test]
fn test_return_without_value() {
    let stmts = parse("return");

    assert!(matches!(stmts[0].kind, StmtKind::Return(None)));
}

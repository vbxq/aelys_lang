use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_syntax::{BinaryOp, Expr, ExprKind, Source, Stmt, StmtKind, UnaryOp};

fn parse(source: &str) -> Vec<Stmt> {
    let src = Source::new("<test>", source);
    let tokens = Lexer::with_source(src.clone()).scan().unwrap();
    Parser::new(tokens, src).parse().unwrap()
}

fn refusal(source: &str) -> String {
    let src = Source::new("<test>", source);
    let tokens = Lexer::with_source(src.clone()).scan().unwrap();
    match Parser::new(tokens, src).parse() {
        Ok(_) => panic!("parsed: {source}"),
        Err(aelys_common::error::AelysError::Multiple(diags)) => {
            format!("{} {}", diags[0].code.clone().unwrap_or_default(), diags[0].message)
        }
        Err(e) => format!("{e:?}"),
    }
}

fn body(source: &str) -> Vec<Stmt> {
    let stmts = parse(&format!("fn f() -> i64 {{ {source} }}"));
    match stmts.into_iter().next().map(|s| s.kind) {
        Some(StmtKind::Function(f)) => f.body,
        _ => panic!("expected a function"),
    }
}

fn tail(source: &str) -> Expr {
    match body(source).pop().map(|s| s.kind) {
        Some(StmtKind::Expression(e)) => e,
        other => panic!("expected a tail expression, got {other:?}"),
    }
}

#[test]
fn statements_need_semicolons_and_the_tail_has_none() {
    let stmts = body("let x = 42; x + 1");
    assert!(matches!(&stmts[0].kind, StmtKind::Let { name, mutable: false, .. } if name == "x"));
    assert!(matches!(&stmts[1].kind, StmtKind::Expression(e) if matches!(e.kind, ExprKind::Binary { .. })));
    assert!(matches!(&body("f(); 1")[0].kind, StmtKind::Semi(_)));
    assert!(matches!(&body("x + 1;")[0].kind, StmtKind::Semi(_)));
    assert!(refusal("fn f() { let a = 1\n let b = 2; }").starts_with("E0110"));
}

#[test]
fn rust_precedence() {
    let e = tail("1 + 2 * 3");
    let ExprKind::Binary { left, op: BinaryOp::Add, right } = e.kind else { panic!() };
    assert!(matches!(left.kind, ExprKind::Int(1)));
    assert!(matches!(right.kind, ExprKind::Binary { op: BinaryOp::Mul, .. }));
    let e = tail("a & 3 == 2");
    assert!(matches!(e.kind, ExprKind::Binary { op: BinaryOp::Eq, .. }));
    let e = tail("a && b || c");
    let ExprKind::Or { left, .. } = e.kind else { panic!() };
    assert!(matches!(left.kind, ExprKind::And { .. }));
    let e = tail("-x as u8");
    let ExprKind::Cast { expr, .. } = e.kind else { panic!() };
    assert!(matches!(expr.kind, ExprKind::Unary { op: UnaryOp::Neg, .. }));
    assert!(refusal("fn f() -> bool { 0 < a < 3 }").starts_with("E0131"));
}

#[test]
fn block_like_statements_need_no_semicolon() {
    let stmts = body("if c { 1 } else { 2 }\n -3");
    assert!(matches!(stmts[0].kind, StmtKind::If { .. }));
    assert!(matches!(&stmts[1].kind, StmtKind::Expression(e) if matches!(e.kind, ExprKind::Int(-3))));
    let e = tail("if c { 1 } else { 2 }.max(3)");
    assert!(matches!(e.kind, ExprKind::Call { .. }));
}

#[test]
fn value_blocks_and_if_without_else() {
    let stmts = body("let x = { let y = 2; y * 3 }; x");
    let StmtKind::Let { initializer, .. } = &stmts[0].kind else { panic!() };
    assert!(matches!(initializer.kind, ExprKind::Block { .. }));
    assert!(refusal("fn f() { let x = { 1; }; }").contains("has no value"));
    assert!(refusal("fn f() { let x = if c { 1 }; }").contains("`else`"));
}

#[test]
fn compound_assignment_binds_an_impure_index_once() {
    let stmts = body("v[g()] += 5; 0");
    let StmtKind::Semi(e) = &stmts[0].kind else { panic!() };
    let ExprKind::Block { stmts, .. } = &e.kind else { panic!("expected a block, got {e:?}") };
    assert!(matches!(stmts[0].kind, StmtKind::Let { .. }));
    let e = tail("x &= 3");
    assert!(matches!(e.kind, ExprKind::Assign { .. }));
}

#[test]
fn closures_and_macros() {
    let e = tail("|x: i64| x + 1");
    assert!(matches!(e.kind, ExprKind::Lambda { .. }));
    let e = tail("|| 7");
    assert!(matches!(e.kind, ExprKind::Lambda { ref params, .. } if params.is_empty()));
    let e = tail("format!(\"{} and {name}\", 1)");
    assert!(matches!(e.kind, ExprKind::FmtString(_)));
    let e = tail("vec![1, 2]");
    assert!(matches!(e.kind, ExprKind::VecLiteral { .. }));
    assert!(refusal("fn f() { println(\"x\"); }").starts_with("E0392"));
}

#[test]
fn items_attributes_and_paths() {
    let stmts = parse("#[inline(always)] #[cold] fn hot() {} use std::str; static mut N: i64 = 0;");
    let StmtKind::Function(f) = &stmts[0].kind else { panic!() };
    assert_eq!(f.decorators.iter().map(|d| d.name.as_str()).collect::<Vec<_>>(), ["inline_always", "cold"]);
    assert!(matches!(stmts[1].kind, StmtKind::Needs(_)));
    assert!(matches!(stmts[2].kind, StmtKind::Let { mutable: true, .. }));
    let e = tail("geo::Shape::Square(2)");
    assert!(matches!(e.kind, ExprKind::EnumVariant { ref enum_name, .. } if enum_name == "geo.Shape"));
}

#[test]
fn old_syntax_is_refused_by_name() {
    let cases = [
        ("fn f() { if a and b {} }", "E0112"),
        ("fn f() { if not a {} }", "E0112"),
        ("needs std.str", "E0115"),
        ("@inline\nfn f() {}", "E0116"),
        ("fn f() { let g = fn(x: i64) -> i64 { x }; }", "E0117"),
        ("fn f() { discard g(); }", "E0118"),
        ("fn f() { let q = g() catch |e| 1; }", "E0119"),
        ("fn f() { for i in 0..10 step 2 {} }", "E0120"),
        ("let g = 1;", "E0122"),
        ("fn f() { i++; }", "E0113"),
        ("fn f() { let v = vec[1]; }", "E0138"),
        ("unsafe extern fn labs(x: i64) -> i64", "E0139"),
        ("struct P { x: i64 };", "E0111"),
        ("#[foo] fn f() {}", "E0130"),
        ("fn f() { let x; }", "E0133"),
    ];
    for (src, code) in cases {
        assert!(refusal(src).starts_with(code), "{src}: {}", refusal(src));
    }
}

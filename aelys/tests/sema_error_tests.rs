use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::{TypeError, TypeInference};
use aelys_syntax::Source;

fn sema_check(code: &str) -> Result<(), Vec<TypeError>> {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    TypeInference::infer_program(stmts, src).map(|_| ())
}

#[test]
fn rejects_binary_type_mismatch() {
    let result = sema_check("fn f() { let x = 1 + \"hello\" }");
    assert!(result.is_err(), "adding i64 and string should be rejected");
}

#[test]
fn rejects_return_type_mismatch() {
    let result = sema_check(r#"fn f() -> i64 { return "text" }"#);
    assert!(result.is_err(), "returning string from i64 function should be rejected");
}

#[test]
fn rejects_if_non_bool_condition() {
    let result = sema_check("fn f() { if 5 { } }");
    assert!(result.is_err(), "if condition must be bool");
}

#[test]
fn rejects_while_non_bool_condition() {
    let result = sema_check(r#"fn f() { while "yes" { } }"#);
    assert!(result.is_err(), "while condition must be bool");
}

#[test]
fn rejects_array_mixed_types() {
    let result = sema_check(r#"fn f() { let a = [1, "two", 3] }"#);
    assert!(result.is_err(), "array with mixed types should be rejected");
}

#[test]
fn rejects_undefined_variable() {
    let result = sema_check("fn f() { let y = x + 1 }");
    assert!(result.is_err(), "undefined variable x should be rejected");
}

#[test]
fn rejects_argument_type_mismatch() {
    let result = sema_check(r#"fn f(x: i64) {} fn g() { f("str") }"#);
    assert!(result.is_err(), "passing string to i64 param should be rejected");
}

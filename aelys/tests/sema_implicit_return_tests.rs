//! bugs where the sema phase doesn't constrain the return type when the function body doesn't end with an expression

use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_syntax::Source;

fn should_fail(code: &str) -> bool {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    TypeInference::infer_program(stmts, src).is_err()
}

fn should_pass(code: &str) {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    match TypeInference::infer_program(stmts, src) {
        Ok(_) => {}
        Err(errors) => {
            for e in &errors {
                eprintln!("  ERROR: {}", e);
            }
            panic!("expected OK, got {} errors", errors.len());
        }
    }
}

// old bug: empty function body with non-void return type

#[test]
fn empty_body_with_return_type_is_rejected() {
    // fn foo() -> i64 {} should fail: empty body can't produce i64
    assert!(should_fail(r#"
fn foo() -> i64 {
}
"#), "empty body with i64 return should be rejected");
}

#[test]
fn empty_body_void_function_is_ok() {
    // fn foo() {} should be fine: return type is inferred as null/void
    should_pass(r#"
fn foo() {
}
"#);
}

#[test]
fn empty_body_with_string_return_is_rejected() {
    assert!(should_fail(r#"
fn bar() -> string {
}
"#), "empty body with string return should be rejected");
}

// old bug: Let as last statement doesn't constrain return type

#[test]
fn let_as_last_stmt_with_return_type_is_rejected() {
    // the let doesn't produce a value, so the function implicitly returns null
    assert!(should_fail(r#"
fn foo() -> i64 {
    let x = 5
}
"#), "let as last statement with i64 return should be rejected");
}

#[test]
fn let_as_last_stmt_void_function_is_ok() {
    should_pass(r#"
fn foo() {
    let x = 5
}
"#);
}

// old bug: while as last statement doesn't constrain return type

#[test]
fn while_as_last_stmt_with_return_type_is_rejected() {
    assert!(should_fail(r#"
fn foo() -> i64 {
    while false {
        return 42
    }
}
"#), "while as last stmt with i64 return should be rejected (loop may not execute)");
}

// old bug: for as last statement doesn't constrain return type

#[test]
fn for_as_last_stmt_with_return_type_is_rejected() {
    assert!(should_fail(r#"
fn foo() -> i64 {
    for i in 0..10 {
        return 42
    }
}
"#), "for as last stmt with i64 return should be rejected");
}
// bug : if without else doesn't constrain return type
#[test]
fn if_without_else_as_last_stmt_with_return_type_is_rejected() {
    assert!(should_fail(r#"
fn foo() -> i64 {
    if true {
        return 42
    }
}
"#), "if without else should be rejected (false path has no return)");
}
// old nested function def as last statement
#[test]
fn nested_function_as_last_stmt_with_return_type_is_rejected() {
    assert!(should_fail(r#"
fn foo() -> i64 {
    fn inner() -> i64 { return 1 }
}
"#), "nested function def as last stmt with i64 return should be rejected");
}

#[test]
fn explicit_return_before_let_is_ok() {
    // the function always returns via explicit return
    should_pass(r#"
fn foo() -> i64 {
    return 42
}
"#);
}

#[test]
fn implicit_return_expression_is_ok() {
    should_pass(r#"
fn foo() -> i64 {
    42
}
"#);
}

#[test]
fn if_else_with_returns_is_ok() {
    should_pass(r#"
fn foo(x: bool) -> i64 {
    if x {
        42
    } else {
        0
    }
}
"#);
}

#[test]
fn if_else_with_explicit_returns_is_ok() {
    should_pass(r#"
fn foo(x: bool) -> i64 {
    if x {
        return 42
    } else {
        return 0
    }
}
"#);
}

// lampda empty body constraints tested implicitly through the same code path
#[test]
fn struct_decl_as_last_stmt_with_return_type_is_rejected() {
    assert!(should_fail(r#"
fn foo() -> i64 {
    struct Point { x: i64, y: i64 }
}
"#), "struct decl as last stmt with i64 return should be rejected");
}

#[test]
fn one_path_returns_other_doesnt_is_rejected() {
    // le if has no else, so the false path falls through with no return
    assert!(should_fail(r#"
fn foo(x: bool) -> i64 {
    if x {
        return 42
    }
    let y = 10
}
"#), "function with let as last stmt should be rejected even with if-return above");
}

#[test]
fn foreach_as_last_stmt_with_return_type_is_rejected() {
    assert!(should_fail(r#"
fn foo() -> i64 {
    let arr = [1, 2, 3]
    for x in arr {
        return x
    }
}
"#), "for-each as last stmt with i64 return should be rejected");
}

#[test]
fn block_ending_with_let_with_return_type_is_rejected() {
    assert!(should_fail(r#"
fn foo() -> i64 {
    {
        let x = 42
    }
}
"#), "block ending with let should be rejected for i64 return");
}

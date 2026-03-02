use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_syntax::Source;

fn sema_ok(code: &str) -> bool {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    TypeInference::infer_program(stmts, src).is_ok()
}

fn sema_error_count(code: &str) -> usize {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    match TypeInference::infer_program(stmts, src) {
        Ok(_) => 0,
        Err(errors) => errors.len(),
    }
}

#[test]
fn a5_bug001_undefined_assign_no_cascade() {
    let count = sema_error_count(
        r#"
fn test() {
    undefined_var = 42
    let x = undefined_var
    let y = undefined_var
}
"#,
    );
    assert!(
        count <= 2,
        ": undefined_var used 3 times should produce at most 2 errors, got {}",
        count
    );
}

#[test]
fn a5_bug001_single_undefined_single_error() {
    let count = sema_error_count(
        r#"
fn test() {
    bad = 1
}
"#,
    );
    assert_eq!(
        count, 1,
        ": single undefined assignment should produce exactly 1 error"
    );
}

#[test]
fn a5_bug002_undefined_read_no_cascade() {
    let count = sema_error_count(
        r#"
fn test() {
    let x = ghost + 1
    let y = ghost * 2
    let z = ghost
}
"#,
    );
    assert!(
        count <= 2,
        "A5-BUG-002: ghost used 3 times should produce at most 2 errors, got {}",
        count
    );
}

#[test]
fn a3_bug001_narrowing_through_variable_i8() {
    assert!(
        sema_ok(
            r#"
fn f() -> i8 {
    let x = 100
    return x
}
"#
        ),
        ": let x = 100; return x in i8 fn should pass sema"
    );
}

#[test]
fn a3_bug001_narrowing_through_variable_i32() {
    assert!(
        sema_ok(
            r#"
fn f() -> i32 {
    let x = 100
    return x
}
"#
        ),
        ": let x = 100; return x in i32 fn should pass sema"
    );
}

#[test]
fn a3_bug001_narrowing_chain() {
    assert!(
        sema_ok(
            r#"
fn f() -> i8 {
    let x = 42
    let y = x
    return y
}
"#
        ),
        ": let y = x where x = 42, return y in i8 fn should pass sema"
    );
}

#[test]
fn a3_bug001_overflow_through_variable() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i8 {
    let x = 200
    return x
}
"#
        ),
        ": let x = 200; return x in i8 fn should FAIL (overflow)"
    );
}

#[test]
fn a3_bug001_mutable_not_tracked() {
    // mutable variables should not be tracked for narrowing because they can be reassigned. this test ensures we don't incorrectly narrow through a mutable variable.
    assert!(
        sema_ok(
            r#"
fn f() -> i64 {
    let mut x = 100
    x = 999999999
    return x
}
"#
        ),
        ": mutable variable should not be narrowed (could be reassigned)"
    );
}

#[test]
fn a3_bug001_negative_literal_through_variable() {
    assert!(
        sema_ok(
            r#"
fn f() -> i8 {
    let x = -1
    return x
}
"#
        ),
        ": let x = -1; return x in i8 fn should pass sema"
    );
}

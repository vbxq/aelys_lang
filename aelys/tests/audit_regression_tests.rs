use aelys_air::lower::lower;
use aelys_air::passes::validate::validate_air;
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

fn air_pipeline_ok(code: &str) -> bool {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let typed = match TypeInference::infer_program(stmts, src) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let air = lower(&typed);
    validate_air(&air).is_ok()
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

#[test]
fn if_else_narrowing_i32() {
    assert!(
        sema_ok(
            r#"
fn f() -> i32 {
    let x = if true { 42 } else { 100 }
    return x
}
"#
        ),
        "if-else with i32-fitting literals assigned to var should pass"
    );
}

#[test]
fn if_else_both_literals_i8() {
    assert!(
        sema_ok(
            r#"
fn f() -> i8 {
    let x = if true { 10 } else { 20 }
    return x
}
"#
        ),
        "if-else with i8-fitting literals should pass"
    );
}

#[test]
fn if_else_direct_return_narrowing() {
    assert!(
        sema_ok(
            r#"
fn f() -> i32 {
    return if true { 1 } else { 2 }
}
"#
        ),
        "direct return of if-else with literals should narrow to i32"
    );
}

#[test]
fn if_else_narrowing_through_variable() {
    assert!(
        sema_ok(
            r#"
fn f() -> i16 {
    let x = if true { 300 } else { 500 }
    return x
}
"#
        ),
        "if-else literals tracked and narrowed through variable for i16"
    );
}

#[test]
fn if_else_overflow_detected() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i8 {
    let x = if true { 10 } else { 200 }
    return x
}
"#
        ),
        "if-else with one branch overflowing i8 should fail"
    );
}

#[test]
fn if_else_same_concrete_type_no_fresh_var() {
    assert!(
        sema_ok(
            r#"
fn f() -> i64 {
    let x = if true { 42 } else { 100 }
    return x
}
"#
        ),
        "if-else with same concrete type should not create a fresh Var"
    );
}

#[test]
fn if_else_different_types_rejected() {
    assert!(
        !sema_ok(
            r#"
fn f() -> i64 {
    let x = if true { 42 } else { "hello" }
    return x
}
"#
        ),
        "if-else with different types should be rejected"
    );
}

#[test]
fn if_without_else_not_affected() {
    assert!(
        sema_ok(
            r#"
fn f() -> i64 {
    let x = 10
    if x > 5 {
        return 1
    }
    return 0
}
"#
        ),
        "if without else should still work"
    );
}

#[test]
fn normal_code_compiles_through_air_pipeline() {
    assert!(
        air_pipeline_ok(
            r#"
fn add(a: i64, b: i64) -> i64 {
    return a + b
}

fn main() -> i64 {
    let x = 42
    let y = add(x, 10)
    return y
}
"#
        ),
        "basic function calls should pass AIR validation"
    );
}

#[test]
fn function_with_if_else_compiles_through_air() {
    assert!(
        air_pipeline_ok(
            r#"
fn f(x: i64) -> i64 {
    let result = if x > 0 { x } else { 0 }
    return result
}
"#
        ),
        "if-else with matching types should pass AIR validation"
    );
}

#[test]
fn multi_function_pipeline_no_var_leak() {
    assert!(
        air_pipeline_ok(
            r#"
fn double(n: i64) -> i64 {
    return n * 2
}

fn is_positive(n: i64) -> bool {
    return n > 0
}

fn main() -> i64 {
    let a = 5
    let b = double(a)
    let flag = is_positive(b)
    if flag {
        return b
    }
    return 0
}
"#
        ),
        "multi-function code should not leak type variables into AIR"
    );
}

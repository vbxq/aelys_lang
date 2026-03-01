//! these are designed to break the Aelys type checker lol
//! each test exercises an edge case or missing validation that could cause miscompilation or unsoundness
//! you get the point.

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

#[test]
fn duplicate_struct_field_names_rejected() {
    assert!(
        should_fail(
            r#"
struct Foo { x: i64, x: string }
fn test() {}
"#
        ),
        "duplicate struct field names should be rejected"
    );
}

#[test]
fn duplicate_struct_field_same_type_rejected() {
    assert!(
        should_fail(
            r#"
struct Foo { x: i64, x: i64 }
fn test() {}
"#
        ),
        "duplicate struct fields with same type should still be rejected"
    );
}

#[test]
fn three_duplicate_fields_rejected() {
    assert!(
        should_fail(
            r#"
struct Bar { a: i64, b: string, a: bool }
fn test() {}
"#
        ),
        "struct with duplicate field 'a' should be rejected"
    );
}

#[test]
fn struct_literal_duplicate_field_rejected() {
    assert!(
        should_fail(
            r#"
struct Foo { x: i64 }
fn test() {
    let f = Foo { x: 1, x: 2 }
}
"#
        ),
        "struct literal with duplicate field should be rejected"
    );
}

#[test]
fn struct_literal_duplicate_field_different_types_rejected() {
    assert!(
        should_fail(
            r#"
struct Foo { x: i64, y: string }
fn test() {
    let f = Foo { x: 1, y: "hi", x: 2 }
}
"#
        ),
        "struct literal with duplicate field (alongside valid fields) should be rejected"
    );
}

#[test]
fn immutable_variable_reassignment_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = 5
    x = 10
}
"#
        ),
        "reassigning an immutable variable should be rejected"
    );
}

#[test]
fn mutable_variable_reassignment_accepted() {
    should_pass(
        r#"
fn test() {
    let mut x = 5
    x = 10
}
"#,
    );
}

#[test]
fn immutable_param_reassignment_rejected() {
    assert!(
        should_fail(
            r#"
fn test(x: i64) {
    x = 10
}
"#
        ),
        "reassigning an immutable parameter should be rejected"
    );
}

#[test]
fn mutable_param_reassignment_accepted() {
    should_pass(
        r#"
fn test(mut x: i64) {
    x = 10
}
"#,
    );
}

#[test]
fn duplicate_function_definitions_rejected() {
    assert!(
        should_fail(
            r#"
fn foo() -> i64 { return 1 }
fn foo() -> string { return "hello" }
fn test() {}
"#
        ),
        "duplicate function definitions should be rejected"
    );
}

#[test]
fn duplicate_function_same_signature_rejected() {
    assert!(
        should_fail(
            r#"
fn foo() -> i64 { return 1 }
fn foo() -> i64 { return 2 }
fn test() {}
"#
        ),
        "duplicate function definitions with same signature should be rejected"
    );
}

#[test]
fn duplicate_parameter_names_rejected() {
    assert!(
        should_fail(
            r#"
fn foo(x: i64, x: string) {}
fn test() {}
"#
        ),
        "duplicate parameter names should be rejected"
    );
}

#[test]
fn error_recovery_doesnt_mask_subsequent_errors() {
    // if the first error poisons a type variable via force_dynamic, a second error on a different variable sharing the same Var should still be caught
    assert!(
        should_fail(
            r#"
fn test() {
    let x: i64 = "bad"
    let y: string = 42
}
"#
        ),
        "both type errors should be caught, not just the first"
    );
}

#[test]
fn error_recovery_multiple_return_mismatches() {
    assert!(
        should_fail(
            r#"
fn test() -> i64 {
    if true {
        return "nope"
    }
    return "also nope"
}
"#
        ),
        "multiple return type mismatches should all be caught"
    );
}

#[test]
fn recursive_function_wrong_return_type() {
    assert!(
        should_fail(
            r#"
fn fib(n: i64) -> string {
    if n <= 1 {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}
fn test() {}
"#
        ),
        "returning i64 from function declared to return string should fail"
    );
}

#[test]
fn empty_array_with_wrong_type_annotation() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x: Array<i64> = ["hello"]
}
"#
        ),
        "string array assigned to Array<i64> should fail"
    );
}

#[test]
fn array_element_type_mismatch_in_function_call() {
    assert!(
        should_fail(
            r#"
fn sum(arr: Array<i64>) -> i64 { return arr[0] }
fn test() {
    let x = sum(["hello", "world"])
}
"#
        ),
        "passing Array<string> to Array<i64> parameter should fail"
    );
}

#[test]
fn for_loop_variable_not_visible_outside() {
    assert!(
        should_fail(
            r#"
fn test() -> i64 {
    for i in 0..10 {}
    return i
}
"#
        ),
        "for loop variable should not be visible outside the loop"
    );
}

#[test]
fn while_body_variable_not_visible_outside() {
    assert!(
        should_fail(
            r#"
fn test() -> i64 {
    while true {
        let x = 42
    }
    return x
}
"#
        ),
        "variable defined in while body should not be visible outside"
    );
}

#[test]
fn if_expr_different_branch_types_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = if true { 42 } else { "hello" }
}
"#
        ),
        "if expression with different branch types should be rejected"
    );
}

#[test]
fn too_many_arguments_rejected() {
    assert!(
        should_fail(
            r#"
fn add(a: i64, b: i64) -> i64 { return a + b }
fn test() {
    let x = add(1, 2, 3)
}
"#
        ),
        "calling function with too many arguments should fail"
    );
}

#[test]
fn too_few_arguments_rejected() {
    assert!(
        should_fail(
            r#"
fn add(a: i64, b: i64) -> i64 { return a + b }
fn test() {
    let x = add(1)
}
"#
        ),
        "calling function with too few arguments should fail"
    );
}

#[test]
fn string_minus_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = "hello" - "world"
}
"#
        ),
        "string subtraction should be rejected"
    );
}

#[test]
fn bool_arithmetic_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = true + false
}
"#
        ),
        "boolean arithmetic should be rejected"
    );
}

#[test]
fn struct_arithmetic_rejected() {
    assert!(
        should_fail(
            r#"
struct Foo { x: i64 }
fn test() {
    let a = Foo { x: 1 }
    let b = Foo { x: 2 }
    let c = a + b
}
"#
        ),
        "struct arithmetic should be rejected"
    );
}

#[test]
fn comparison_lt_different_types_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = 42 < "hello"
}
"#
        ),
        "comparing i64 < string should be rejected"
    );
}

#[test]
fn comparison_bool_lt_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = true < false
}
"#
        ),
        "boolean comparison with < should be rejected (not numeric)"
    );
}

#[test]
fn index_on_bool_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = true
    let y = x[0]
}
"#
        ),
        "indexing a bool should be rejected"
    );
}

#[test]
fn index_on_integer_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = 42
    let y = x[0]
}
"#
        ),
        "indexing an integer should be rejected"
    );
}

#[test]
fn member_access_on_integer_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = 42
    let y = x.foo
}
"#
        ),
        "member access on integer should be rejected"
    );
}

#[test]
fn unknown_struct_field_rejected() {
    assert!(
        should_fail(
            r#"
struct Foo { x: i64 }
fn test() {
    let f = Foo { x: 1 }
    let y = f.nonexistent
}
"#
        ),
        "accessing nonexistent field should be rejected"
    );
}

#[test]
fn string_to_int_cast_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = "hello" as i64
}
"#
        ),
        "casting string to i64 should be rejected"
    );
}

#[test]
fn struct_to_int_cast_rejected() {
    assert!(
        should_fail(
            r#"
struct Foo { x: i64 }
fn test() {
    let f = Foo { x: 1 }
    let y = f as i64
}
"#
        ),
        "casting struct to i64 should be rejected"
    );
}

#[test]
fn bitwise_on_float_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = 1.5 & 2.5
}
"#
        ),
        "bitwise AND on floats should be rejected"
    );
}

#[test]
fn bitwise_on_string_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = "a" | "b"
}
"#
        ),
        "bitwise OR on strings should be rejected"
    );
}

#[test]
fn logical_and_on_integers_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = 1 and 2
}
"#
        ),
        "logical AND on integers should be rejected"
    );
}

#[test]
fn logical_or_on_strings_rejected() {
    assert!(
        should_fail(
            r#"
fn test() {
    let x = "a" or "b"
}
"#
        ),
        "logical OR on strings should be rejected"
    );
}

#[test]
fn inner_function_captures_dont_leak() {
    should_pass(
        r#"
fn outer() -> i64 {
    let x = 10
    fn inner() -> i64 {
        return x
    }
    return inner()
}
fn test() {}
"#,
    );
}

#[test]
fn shadowing_same_type_passes() {
    should_pass(
        r#"
fn test() -> i64 {
    let x = 5
    let x = 10
    return x
}
"#,
    );
}

#[test]
fn shadowing_different_type_passes() {
    should_pass(
        r#"
fn test() -> string {
    let x: i64 = 5
    let x: string = "hello"
    return x
}
"#,
    );
}

#[test]
fn nested_block_scoping_passes() {
    should_pass(
        r#"
fn test() -> i64 {
    let x = 5
    if true {
        let y = x + 1
    }
    return x
}
"#,
    );
}

#[test]
fn recursive_function_correct_types() {
    should_pass(
        r#"
fn factorial(n: i64) -> i64 {
    if n <= 1 {
        return 1
    }
    return n * factorial(n - 1)
}
fn test() {}
"#,
    );
}

#[test]
fn multiple_returns_same_type() {
    should_pass(
        r#"
fn abs(x: i64) -> i64 {
    if x < 0 {
        return 0 - x
    }
    return x
}
fn test() {}
"#,
    );
}

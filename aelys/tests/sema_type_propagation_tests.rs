use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::{ResolvedType, TypeInference, TypedExprKind, TypedStmtKind};
use aelys_syntax::Source;

fn should_fail(code: &str) -> bool {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    TypeInference::infer_program(stmts, src).is_err()
}

fn infer_ok(code: &str) -> aelys_sema::TypedProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    TypeInference::infer_program(stmts, src).expect("inference failed")
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
fn generic_id_wrong_return_type() {
    // id<T> returns T, but caller expects string while passing i64
    assert!(should_fail(
        r#"
fn id<T>(x: T) -> T { return x }
fn test() -> string {
    return id(42)
}
"#
    ));
}

#[test]
fn generic_id_correct_return_type() {
    should_pass(
        r#"
fn id<T>(x: T) -> T { return x }
fn test() -> i64 {
    return id(42)
}
"#,
    );
}

#[test]
fn generic_two_params_swap_return_wrong() {
    // swap<A,B> returns B. Passing (string, i64) returns i64, but we expect string
    assert!(should_fail(
        r#"
fn swap<A, B>(a: A, b: B) -> B { return b }
fn test() -> string {
    return swap("hello", 42)
}
"#
    ));
}

#[test]
fn generic_two_params_swap_return_correct() {
    should_pass(
        r#"
fn swap<A, B>(a: A, b: B) -> B { return b }
fn test() -> i64 {
    return swap("hello", 42)
}
"#,
    );
}

#[test]
fn generic_called_twice_with_different_types() {
    // Each call site should get its own instantiation
    should_pass(
        r#"
fn id<T>(x: T) -> T { return x }
fn test() {
    let a: i64 = id(42)
    let b: string = id("hello")
}
"#,
    );
}

#[test]
fn generic_called_twice_one_wrong() {
    // Second call returns i64 but expects string
    assert!(should_fail(
        r#"
fn id<T>(x: T) -> T { return x }
fn test() {
    let a: i64 = id(42)
    let b: string = id(99)
}
"#
    ));
}

#[test]
fn generic_same_type_param_used_twice_in_params() {
    // first<T>(a: T, b: T) -> T: both args must have the same type
    assert!(should_fail(
        r#"
fn first<T>(a: T, b: T) -> T { return a }
fn test() -> i64 {
    return first(42, "hello")
}
"#
    ));
}

#[test]
fn generic_same_type_param_both_correct() {
    should_pass(
        r#"
fn first<T>(a: T, b: T) -> T { return a }
fn test() -> i64 {
    return first(42, 99)
}
"#,
    );
}

#[test]
fn if_else_branches_different_types_rejected() {
    assert!(should_fail(
        r#"
fn test(x: bool) -> i64 {
    if x {
        42
    } else {
        "hello"
    }
}
"#
    ));
}

#[test]
fn nested_if_else_type_mismatch() {
    assert!(should_fail(
        r#"
fn test(a: bool, b: bool) -> i64 {
    if a {
        if b {
            42
        } else {
            "wrong"
        }
    } else {
        0
    }
}
"#
    ));
}

#[test]
fn let_annotation_mismatch_with_string() {
    assert!(should_fail(
        r#"
fn test() {
    let x: i64 = "hello"
}
"#
    ));
}

#[test]
fn let_annotation_mismatch_with_bool() {
    assert!(should_fail(
        r#"
fn test() {
    let x: i64 = true
}
"#
    ));
}

#[test]
fn let_annotation_correct() {
    should_pass(
        r#"
fn test() {
    let x: i64 = 42
}
"#,
    );
}

#[test]
fn assign_wrong_type_to_variable() {
    assert!(should_fail(
        r#"
fn test() {
    let mut x: i64 = 42
    x = "hello"
}
"#
    ));
}

#[test]
fn assign_bool_to_i64_variable() {
    assert!(should_fail(
        r#"
fn test() {
    let mut x: i64 = 0
    x = true
}
"#
    ));
}

#[test]
fn wrong_number_of_args_extra() {
    assert!(should_fail(
        r#"
fn add(a: i64, b: i64) -> i64 { return a + b }
fn test() -> i64 {
    return add(1, 2, 3)
}
"#
    ));
}

#[test]
fn wrong_number_of_args_fewer() {
    assert!(should_fail(
        r#"
fn add(a: i64, b: i64) -> i64 { return a + b }
fn test() -> i64 {
    return add(1)
}
"#
    ));
}

#[test]
fn constraint_chain_through_variables() {
    // x = 42 (i64), y = x (i64), then y used as string → error
    assert!(should_fail(
        r#"
fn consume_string(s: string) {}
fn test() {
    let x = 42
    let y = x
    consume_string(y)
}
"#
    ));
}

#[test]
fn constraint_chain_correct() {
    should_pass(
        r#"
fn double(n: i64) -> i64 { return n + n }
fn test() {
    let x = 42
    let y = double(x)
    let z = double(y)
}
"#,
    );
}

#[test]
fn block_implicit_return_type_mismatch() {
    assert!(should_fail(
        r#"
fn test() -> i64 {
    {
        "hello"
    }
}
"#
    ));
}

#[test]
fn block_implicit_return_correct() {
    should_pass(
        r#"
fn test() -> i64 {
    {
        42
    }
}
"#,
    );
}

#[test]
fn comparison_between_incompatible_types() {
    assert!(should_fail(
        r#"
fn test() -> bool {
    return 42 > "hello"
}
"#
    ));
}

#[test]
fn comparison_bool_with_integer() {
    assert!(should_fail(
        r#"
fn test() -> bool {
    return true > 5
}
"#
    ));
}

#[test]
fn addition_string_and_bool() {
    assert!(should_fail(
        r#"
fn test() {
    let x = "hello" + true
}
"#
    ));
}

#[test]
fn subtraction_on_booleans() {
    assert!(should_fail(
        r#"
fn test() {
    let x = true - false
}
"#
    ));
}

#[test]
fn struct_field_wrong_type_in_init() {
    assert!(should_fail(
        r#"
struct Point { x: i64, y: i64 }
fn test() {
    let p = Point { x: "hello", y: 0 }
}
"#
    ));
}

#[test]
fn struct_field_access_used_as_wrong_type() {
    assert!(should_fail(
        r#"
struct Point { x: i64, y: i64 }
fn consume_string(s: string) {}
fn test() {
    let p = Point { x: 1, y: 2 }
    consume_string(p.x)
}
"#
    ));
}

#[test]
fn array_used_where_i64_expected() {
    assert!(should_fail(
        r#"
fn double(n: i64) -> i64 { return n + n }
fn test() {
    let arr = [1, 2, 3]
    double(arr)
}
"#
    ));
}

#[test]
fn array_element_used_correctly() {
    should_pass(
        r#"
fn double(n: i64) -> i64 { return n + n }
fn test() {
    let arr = [1, 2, 3]
    double(arr[0])
}
"#,
    );
}

#[test]
fn multiple_independent_errors() {
    // both lines should produce errors, second shouldn't be swallowed
    assert!(should_fail(
        r#"
fn test() {
    let x: i64 = "hello"
    let y: string = 42
}
"#
    ));
}

#[test]
fn function_result_plus_number_rejected() {
    // can't add a function call result that is string to a number
    assert!(should_fail(
        r#"
fn greet() -> string { return "hi" }
fn test() -> i64 {
    return greet() + 10
}
"#
    ));
}

#[test]
fn function_result_used_correctly() {
    should_pass(
        r#"
fn double(n: i64) -> i64 { return n + n }
fn test() -> i64 {
    return double(5) + 10
}
"#,
    );
}

#[test]
fn void_function_result_used_as_value() {
    assert!(should_fail(
        r#"
fn do_nothing() {}
fn test() -> i64 {
    return do_nothing()
}
"#
    ));
}

#[test]
fn void_function_called_correctly() {
    should_pass(
        r#"
fn do_nothing() {}
fn test() {
    do_nothing()
}
"#,
    );
}

#[test]
fn recursive_function_returns_wrong_type_in_base() {
    assert!(should_fail(
        r#"
fn countdown(n: i64) -> i64 {
    if n == 0 {
        return "done"
    }
    return countdown(n - 1)
}
"#
    ));
}

#[test]
fn recursive_function_correct() {
    should_pass(
        r#"
fn factorial(n: i64) -> i64 {
    if n <= 1 {
        return 1
    }
    return n * factorial(n - 1)
}
"#,
    );
}

/// VecLiteral.element_type must be substituted
/// element_type was cloned verbatim through the substitution pass, so a type annotation that resolved through inference could remain stale.
#[test]
fn vec_literal_element_type_is_substituted() {
    let program = infer_ok(
        r#"
fn test() {
    let v = vec<i32>[1, 2, 3]
}
"#,
    );
    // walk the AST to find the VecLiteral and check its element_type
    for stmt in &program.stmts {
        if let TypedStmtKind::Function(func) = &stmt.kind {
            for body_stmt in &func.body {
                if let TypedStmtKind::Let { initializer, .. } = &body_stmt.kind {
                    if let TypedExprKind::VecLiteral { element_type, .. } = &initializer.kind {
                        let et = element_type
                            .as_ref()
                            .expect("element_type should be Some for annotated vec");
                        assert_eq!(
                            *et,
                            ResolvedType::I32,
                            "VecLiteral.element_type should be I32 after substitution, got {:?}",
                            et
                        );
                        return;
                    }
                }
            }
        }
    }
    panic!("did not find VecLiteral in typed AST");
}

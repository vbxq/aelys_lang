//! tests for type annotation validation, ensures unknown types in annotations are properly rejected rather than silently accepted

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
fn struct_field_unknown_type_rejected() {
    assert!(
        should_fail(r#"
struct Foo { x: Nonexistent }
fn test() {}
"#),
        "struct field with unknown type should be rejected"
    );
}

#[test]
fn struct_field_valid_types_accepted() {
    should_pass(r#"
struct Foo { x: i64, y: string, z: bool }
fn test() {}
"#);
}

#[test]
fn struct_field_nested_struct_type_accepted() {
    should_pass(r#"
struct Inner { value: i64 }
struct Outer { inner: Inner }
fn test() {}
"#);
}

#[test]
fn generic_struct_field_type_param_accepted() {
    // T is a type parameter; should not be rejected
    should_pass(r#"
struct Wrapper<T> { value: T }
fn test() {}
"#);
}

#[test]
fn struct_field_array_of_known_type_accepted() {
    should_pass(r#"
struct Matrix { data: Array<i64> }
fn test() {}
"#);
}

#[test]
fn cast_to_unknown_type_rejected() {
    assert!(
        should_fail(r#"
fn test() {
    let x = 42 as Nonexistent
}
"#),
        "cast to unknown type should be rejected"
    );
}

#[test]
fn cast_to_valid_type_accepted() {
    should_pass(r#"
fn test() {
    let x = 42 as i32
}
"#);
}

#[test]
fn cast_to_bool_from_int_accepted() {
    should_pass(r#"
fn test() {
    let x = 1 as bool
}
"#);
}

#[test]
fn let_unknown_type_annotation_rejected() {
    assert!(
        should_fail(r#"
fn test() {
    let x: Nonexistent = 42
}
"#),
        "let with unknown type annotation should be rejected"
    );
}

#[test]
fn let_valid_type_annotation_accepted() {
    should_pass(r#"
fn test() {
    let x: i64 = 42
}
"#);
}

#[test]
fn function_param_unknown_type_rejected() {
    assert!(
        should_fail(r#"
fn foo(x: Nonexistent) {}
fn test() {}
"#),
        "function param with unknown type should be rejected"
    );
}

#[test]
fn function_return_unknown_type_rejected() {
    assert!(
        should_fail(r#"
fn foo() -> Nonexistent { }
fn test() {}
"#),
        "function with unknown return type should be rejected"
    );
}

#[test]
fn generic_function_type_param_not_rejected() {
    should_pass(r#"
fn id<T>(x: T) -> T { return x }
fn test() { let y = id(42) }
"#);
}

#[test]
fn generic_function_multiple_type_params_not_rejected() {
    should_pass(r#"
fn pair<A, B>(a: A, b: B) -> A { return a }
fn test() { let y = pair(1, "hi") }
"#);
}

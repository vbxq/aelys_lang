/// finalization, error recovery, struct validation, for-each validation, and env scoping tests bugs

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

fn sema_err(code: &str) -> bool {
    !sema_ok(code)
}

// when error recovery forces types to Dynamic, Vec inner types must
// also be forced. If not, unresolved vars leak into the typed AST

#[test]
fn force_dynamic_handles_vec_inner_type() {
    // this should produce a type error (string + i64 inside a vec element), and error recovery should force the Vec's inner var to Dynamic without crashing or leaving orphaned vars
    let result = sema_ok(
        r#"
fn f() {
    let v = vec[1, "hello"]
}
"#,
    );
    // should fail because array elements are mixed types
    assert!(!result, "vec with mixed element types should be rejected");
}

// for-each should reject non-iterable types like i64, bool, etc.

#[test]
fn rejects_for_each_on_integer() {
    assert!(
        sema_err(
            r#"
fn f() {
    for x in 42 {
        let y = x
    }
}
"#
        ),
        "for-each over an integer should be rejected"
    );
}

#[test]
fn rejects_for_each_on_bool() {
    assert!(
        sema_err(
            r#"
fn f() {
    for x in true {
        let y = x
    }
}
"#
        ),
        "for-each over a bool should be rejected"
    );
}

#[test]
fn rejects_for_each_on_struct() {
    assert!(
        sema_err(
            r#"
struct Point { x: i64, y: i64 }
fn f() {
    let p = Point { x: 1, y: 2 }
    for x in p {
        let y = x
    }
}
"#
        ),
        "for-each over a struct should be rejected"
    );
}

#[test]
fn for_each_on_array_is_ok() {
    assert!(
        sema_ok(
            r#"
fn f() {
    let arr = [1, 2, 3]
    for x in arr {
        let y = x
    }
}
"#
        ),
        "for-each over array should compile"
    );
}

#[test]
fn for_each_on_string_is_ok() {
    assert!(
        sema_ok(
            r#"
fn f() {
    for c in "hello" {
        let x = c
    }
}
"#
        ),
        "for-each over string should compile"
    );
}

// struct literals should validate that all fields exist and are provided.

#[test]
fn rejects_struct_literal_with_unknown_field() {
    assert!(
        sema_err(
            r#"
struct Point { x: i64, y: i64 }
fn f() {
    let p = Point { x: 1, y: 2, z: 3 }
}
"#
        ),
        "struct literal with unknown field 'z' should be rejected"
    );
}

#[test]
fn rejects_struct_literal_with_missing_field() {
    assert!(
        sema_err(
            r#"
struct Point { x: i64, y: i64 }
fn f() {
    let p = Point { x: 1 }
}
"#
        ),
        "struct literal missing field 'y' should be rejected"
    );
}

#[test]
fn struct_literal_with_all_fields_is_ok() {
    assert!(
        sema_ok(
            r#"
struct Point { x: i64, y: i64 }
fn f() {
    let p = Point { x: 1, y: 2 }
}
"#
        ),
        "struct literal with all fields should compile"
    );
}


// Var -> Dynamic conversion
// unresolved type variables should be converted to Dynamic (or at least not left as Var in the final typed AST)

#[test]
fn empty_array_type_is_resolved() {
    // empty array has type Array(Var(N), 0). After finalization the inner type should be Dynamic, not an unresolved vAR
    assert!(
        sema_ok(
            r#"
fn f() {
    let arr = []
}
"#
        ),
        "empty array literal should compile"
    );
}

#[test]
fn unannotated_function_return_compiles() {
    // functions without return type annotation gets Var for return type, should be resolved to Null or Dynamic, not left as Var
    assert!(
        sema_ok(
            r#"
fn noop() {
    let x = 1
}
"#
        ),
        "function without return type should compile"
    );
}

// TODO: i got that wrong.
/// fn bad() -> i32 { return "oops" }
///
/// invert the message

#[test]
fn nested_struct_field_access_validates_types() {
    assert!(
        sema_ok(
            r#"
struct Inner { val: i64 }
struct Outer { inner: Inner }
fn test() -> i64 {
    let i = Inner { val: 10 }
    let o = Outer { inner: i }
    return o.inner.val
}
"#
        ),
        "nested struct field access should compile"
    );
}

#[test]
fn rejects_field_access_on_wrong_struct() {
    assert!(
        sema_err(
            r#"
struct A { x: i64 }
struct B { y: i64 }
fn f() -> i64 {
    let a = A { x: 1 }
    return a.y
}
"#
        ),
        "accessing field 'y' on struct A should be rejected"
    );
}

// slicing should not blindly return the object's type
#[test]
fn rejects_slice_on_integer() {
    // `42[0..1]` should be rejected because i64 is not sliceable
    assert!(
        sema_err(
            r#"
fn f() -> i64 {
    return 42[0..1]
}
"#
        ),
        "slicing an integer should be rejected"
    );
}

#[test]
fn rejects_index_on_bool() {
    assert!(
        sema_err(
            r#"
fn f() {
    let b = true
    let x = b[0]
}
"#
        ),
        "indexing a bool should be rejected"
    );
}

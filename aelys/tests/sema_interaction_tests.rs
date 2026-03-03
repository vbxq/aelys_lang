/// Tests targeting deep interactions between sema components

use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_syntax::Source;
use std::collections::HashSet;

fn sema_ok(code: &str) -> bool {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    TypeInference::infer_program(stmts, src).is_ok()
}

fn sema_ok_with_builtins(code: &str) -> bool {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let builtins: HashSet<String> = ["print", "println"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    TypeInference::infer_program_with_imports(stmts, src, Default::default(), builtins).is_ok()
}

fn sema_err(code: &str) -> bool {
    !sema_ok(code)
}

#[test]
fn constraint_chain_through_multiple_variables() {
    // a = b, b = c, c = i64, should all resolve to i64
    assert!(
        sema_ok(
            r#"
fn f() -> i64 {
    let c: i64 = 42
    let b = c
    let a = b
    return a
}
"#
        ),
        "chained variable assignments should resolve types correctly"
    );
}

#[test]
fn constraint_conflict_detected() {
    // a used as both i64 and string, should error
    assert!(
        sema_err(
            r#"
fn f(a: i64) -> string {
    return a
}
"#
        ),
        "returning i64 as string should be rejected"
    );
}

#[test]
fn generic_function_called_with_different_types() {
    // generic function called with i64 and string, both should work
    assert!(
        sema_ok(
            r#"
fn id<T>(x: T) -> T { return x }
fn test() -> i64 {
    let a = id(42)
    return a
}
"#
        ),
        "generic function with concrete call should compile"
    );
}

#[test]
fn struct_field_type_resolves_through_constraint() {
    // field access on struct should return the correct type
    assert!(
        sema_ok(
            r#"
struct Vec2 { x: f64, y: f64 }
fn length(v: Vec2) -> f64 {
    return v.x * v.x + v.y * v.y
}
"#
        ),
        "struct field access should resolve to declared field type"
    );
}

#[test]
fn struct_field_used_in_binary_op() {
    // using struct field in binary op with wrong type should error
    assert!(
        sema_err(
            r#"
struct Foo { x: i64 }
fn f() -> string {
    let foo = Foo { x: 42 }
    return foo.x
}
"#
        ),
        "returning i64 field as string should be rejected"
    );
}

#[test]
fn struct_field_narrowing() {
    // struct fields should narrow integer literals
    assert!(
        sema_ok(
            r#"
struct Small { val: i32 }
fn f() -> i32 {
    let s = Small { val: 10 }
    return s.val
}
"#
        ),
        "struct with i32 field and literal should compile"
    );
}

#[test]
fn array_of_structs_field_access() {
    assert!(
        sema_ok(
            r#"
struct Item { val: i64 }
fn get_val(items: Array<Item>, idx: i64) -> i64 {
    return items[idx].val
}
"#
        ),
        "accessing field on array-indexed struct should compile"
    );
}

#[test]
fn array_element_type_mismatch_in_struct() {
    assert!(
        sema_err(
            r#"
fn f() {
    let arr = [1, 2, 3]
    let s: string = arr[0]
}
"#
        ),
        "assigning i64 array element to string should be rejected"
    );
}

#[test]
fn annotated_vec_rejects_incompatible_element_type() {
    assert!(
        sema_err(
            r#"
fn f() {
    let v = vec<i64>["hello"]
}
"#
        ),
        "vec<i64> must reject string element"
    );
}

#[test]
fn closure_capture_type_resolution() {
    assert!(
        sema_ok(
            r#"
fn apply(f: fn(i64) -> i64, x: i64) -> i64 { return f(x) }
fn test() -> i64 {
    let offset: i64 = 10
    let add = fn(x: i64) -> i64 { return x + offset }
    return apply(add, 5)
}
"#
        ),
        "closure capturing i64 should resolve correctly"
    );
}

#[test]
fn closure_return_type_mismatch() {
    assert!(
        sema_err(
            r#"
fn test() {
    let f = fn(x: i64) -> string { return x }
}
"#
        ),
        "closure returning i64 as string should be rejected"
    );
}

#[test]
fn error_in_struct_field_doesnt_crash_other_functions() {
    // even with a type error in one function, sema should still report it and not crash
    assert!(
        sema_err(
            r#"
struct Point { x: i64, y: i64 }
fn bad() -> i64 {
    let p = Point { x: 1, y: 2 }
    return p.z
}
fn good() -> i64 {
    return 42
}
"#
        ),
        "accessing unknown field should be rejected"
    );
}
#[test]
fn narrowing_in_if_branches() {
    assert!(
        sema_ok(
            r#"
fn f(cond: bool) -> i32 {
    if cond {
        return 1
    }
    return 0
}
"#
        ),
        "i32 literal narrowing should work in if branches"
    );
}

#[test]
fn narrowing_array_of_i32() {
    assert!(
        sema_ok(
            r#"
fn f() -> i32 {
    let arr: Array<i32> = [1, 2, 3]
    return arr[0]
}
"#
        ),
        "Array<i32> with literal elements should narrow correctly"
    );
}

#[test]
fn narrowing_in_while_body() {
    assert!(
        sema_ok(
            r#"
fn f() -> i32 {
    let mut x: i32 = 0
    while x < 10 {
        x = x + 1
    }
    return x
}
"#
        ),
        "i32 assignment in while body should narrow"
    );
}

#[test]
fn dynamic_function_accepts_any_type() {
    assert!(
        sema_ok_with_builtins(
            r#"
fn test() {
    println("hello")
    println(42)
    println(true)
}
"#
        ),
        "Dynamic-typed function should accept any argument type"
    );
}

#[test]
fn recursive_function_return_type() {
    assert!(
        sema_ok(
            r#"
fn fib(n: i64) -> i64 {
    if n < 2 {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}
"#
        ),
        "recursive function with i64 return should resolve"
    );
}

#[test]
fn recursive_function_return_type_mismatch() {
    assert!(
        sema_err(
            r#"
fn fib(n: i64) -> string {
    if n < 2 {
        return n
    }
    return fib(n - 1)
}
"#
        ),
        "returning i64 from string function should be rejected even with recursion"
    );
}

#[test]
fn index_assign_type_mismatch() {
    assert!(
        sema_err(
            r#"
fn f() {
    let mut arr = [1, 2, 3]
    arr[0] = "hello"
}
"#
        ),
        "assigning string to i64 array should be rejected"
    );
}

#[test]
fn cast_preserves_type() {
    assert!(
        sema_ok(
            r#"
fn f(x: i64) -> f64 {
    return x as f64
}
"#
        ),
        "cast i64 to f64 should compile"
    );
}

#[test]
fn cast_chain_type_changes() {
    assert!(
        sema_ok(
            r#"
fn f(x: i64) -> i8 {
    return (x as i32) as i8
}
"#
        ),
        "chained casts should compile"
    );
}

#[test]
fn empty_void_function() {
    assert!(
        sema_ok("fn noop() -> void {}"),
        "empty void function should compile"
    );
}

#[test]
fn empty_function_no_annotation() {
    assert!(
        sema_ok("fn noop() {}"),
        "empty function without return annotation should compile"
    );
}

#[test]
fn multiple_returns_different_literal_types() {
    assert!(
        sema_err(
            r#"
fn f(cond: bool) -> i64 {
    if cond {
        return "hello"
    }
    return 42
}
"#
        ),
        "one return path with wrong type should be rejected"
    );
}


#[test]
fn for_loop_iterator_is_i64() {
    assert!(
        sema_ok(
            r#"
fn sum_to(n: i64) -> i64 {
    let mut total: i64 = 0
    for i in 0..n {
        total = total + i
    }
    return total
}
"#
        ),
        "for loop with range should have i64 iterator"
    );
}

#[test]
fn nested_function_has_own_scope() {
    assert!(
        sema_ok(
            r#"
fn outer() -> i64 {
    fn inner(x: i64) -> i64 { return x + 1 }
    return inner(41)
}
"#
        ),
        "nested function should have its own scope"
    );
}

#[test]
fn string_equality_comparison() {
    assert!(
        sema_ok(
            r#"
fn eq(a: string, b: string) -> bool {
    return a == b
}
"#
        ),
        "string equality should return bool"
    );
}

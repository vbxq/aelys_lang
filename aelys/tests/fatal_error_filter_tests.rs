/// Tests that programs which SHOULD compile aren't falsely rejected
/// by the all-fatal error filter in sema/entry.rs.
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
    let builtins: HashSet<String> = ["print", "println"].iter().map(|s| s.to_string()).collect();
    TypeInference::infer_program_with_imports(stmts, src, Default::default(), builtins).is_ok()
}

#[test]
fn generic_identity_function() {
    assert!(
        sema_ok("fn id<T>(x: T) -> T { return x }"),
        "generic identity function should compile"
    );
}

#[test]
fn generic_with_concrete_call() {
    assert!(
        sema_ok(
            r#"
fn id<T>(x: T) -> T { return x }
fn use_it() -> i64 { return id(42) }
"#
        ),
        "calling generic fn with concrete arg should compile"
    );
}

#[test]
fn generic_with_multiple_instantiations() {
    assert!(
        sema_ok(
            r#"
fn first<T>(a: T, b: T) -> T { return a }
fn test() -> i64 {
    let a = first(1, 2)
    return a
}
"#
        ),
        "multiple generic instantiations should compile"
    );
}

#[test]
fn simple_closure() {
    assert!(
        sema_ok(
            r#"
fn apply(f: fn(i64) -> i64, x: i64) -> i64 { return f(x) }
fn test() -> i64 {
    let double = fn(x: i64) -> i64 { return x * 2 }
    return apply(double, 5)
}
"#
        ),
        "simple closure should compile"
    );
}

#[test]
fn closure_capturing_local() {
    assert!(
        sema_ok(
            r#"
fn test() -> i64 {
    let offset: i64 = 10
    let add_offset = fn(x: i64) -> i64 { return x + offset }
    return add_offset(5)
}
"#
        ),
        "closure capturing local should compile"
    );
}

#[test]
fn recursive_factorial() {
    assert!(
        sema_ok(
            r#"
fn factorial(n: i64) -> i64 {
    if n < 2 {
        return 1
    }
    return n * factorial(n - 1)
}
"#
        ),
        "recursive factorial should compile"
    );
}

#[test]
fn mutual_recursion_style() {
    // not truly mutual (would need forward decl) but tests that calling another function from within a function works
    assert!(
        sema_ok(
            r#"
fn is_even(n: i64) -> bool { return n == 0 }
fn is_odd(n: i64) -> bool {
    if n == 0 { return false }
    return is_even(n - 1)
}
"#
        ),
        "pseudo-mutual recursion should compile"
    );
}

#[test]
fn struct_with_methods_style() {
    assert!(
        sema_ok(
            r#"
struct Point { x: i64, y: i64 }
fn point_sum(p: Point) -> i64 {
    return p.x + p.y
}
fn test() -> i64 {
    let p = Point { x: 1, y: 2 }
    return point_sum(p)
}
"#
        ),
        "struct creation and field access should compile"
    );
}

#[test]
fn nested_struct() {
    assert!(
        sema_ok(
            r#"
struct Inner { val: i64 }
struct Outer { inner: Inner }
fn test() -> i64 {
    let inner = Inner { val: 42 }
    let outer = Outer { inner: inner }
    return outer.inner.val
}
"#
        ),
        "nested struct should compile"
    );
}

#[test]
fn string_len() {
    assert!(
        sema_ok(
            r#"
fn str_len(s: string) -> i64 { return s.len }
"#
        ),
        "string .len should compile"
    );
}

#[test]
fn string_concatenation() {
    assert!(
        sema_ok(
            r#"
fn greet(name: string) -> string { return "Hello, " + name }
"#
        ),
        "string concatenation should compile"
    );
}

#[test]
fn function_as_parameter() {
    assert!(
        sema_ok(
            r#"
fn apply(f: fn(i64) -> i64, x: i64) -> i64 { return f(x) }
fn double(x: i64) -> i64 { return x * 2 }
fn test() -> i64 { return apply(double, 5) }
"#
        ),
        "passing function as parameter should compile"
    );
}

#[test]
fn nested_if_with_returns() {
    assert!(
        sema_ok(
            r#"
fn classify(x: i64) -> i64 {
    if x > 0 {
        if x > 100 {
            return 3
        }
        return 2
    }
    if x == 0 {
        return 1
    }
    return 0
}
"#
        ),
        "nested if with multiple returns should compile"
    );
}

#[test]
fn while_with_early_return() {
    assert!(
        sema_ok(
            r#"
fn find(arr: Array<i64>, n: i64, target: i64) -> i64 {
    let mut i: i64 = 0
    while i < n {
        if arr[i] == target {
            return i
        }
        i = i + 1
    }
    return -1
}
"#
        ),
        "while with early return should compile"
    );
}

#[test]
fn rejects_string_minus_string() {
    assert!(
        !sema_ok(r#"fn f() -> string { return "a" - "b" }"#),
        "string subtraction should be rejected"
    );
}

#[test]
fn rejects_bool_arithmetic() {
    assert!(
        !sema_ok("fn f() -> bool { return true + false }"),
        "bool addition should be rejected"
    );
}

#[test]
fn void_function_no_return() {
    assert!(
        sema_ok(
            r#"
fn noop() -> void {
    let x: i64 = 1
}
"#
        ),
        "void function without return should compile"
    );
}

#[test]
fn dynamic_type_unifies_with_anything() {
    // println is registered as Dynamic via bootstrap builtins
    // TODO: REMOVE WHEN BOOTSTRAPPING IS DONE.
    assert!(
        sema_ok_with_builtins(
            r#"
fn test() { println("hello") }
"#
        ),
        "Dynamic-typed println should accept string"
    );
}

#[test]
fn println_does_not_satisfy_non_void_return() {
    assert!(
        !sema_ok_with_builtins(
            r#"
fn main() -> i64 {
    println("hello")
}
"#
        ),
        "println should not be accepted as implicit i64 return value"
    );
}

#[test]
fn explicit_cast_i32_to_f64() {
    assert!(
        sema_ok("fn f(x: i32) -> f64 { return x as f64 }"),
        "explicit cast should compile"
    );
}

#[test]
fn explicit_cast_chain() {
    assert!(
        sema_ok("fn f(x: i64) -> i8 { return (x as i32) as i8 }"),
        "chained casts should compile"
    );
}

#[test]
fn generic_with_struct_name_collision_rejected() {
    // struct T shadows type param T in instantiate_type_params, so identity(42) is a type error
    assert!(
        !sema_ok(
            r#"
struct T { value: i64 }
fn identity<T>(x: T) -> T { return x }
fn test() -> i64 { return identity(42) }
"#
        ),
        "struct T shadows type param T, so identity(42) should be rejected"
    );
}

#[test]
fn generic_struct_with_type_param_name_collision_rejected() {
    // struct T shadows type param T in Box<T>, so Box { inner: 42 } is a type error
    assert!(
        !sema_ok(
            r#"
struct T { value: i64 }
struct Box<T> { inner: T }
fn test() -> i64 {
    let b = Box { inner: 42 }
    return b.inner
}
"#
        ),
        "struct T shadows type param T in generic struct, should be rejected"
    );
}

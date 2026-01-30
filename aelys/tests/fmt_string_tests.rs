use aelys::run;

fn run_ok(source: &str) -> aelys_runtime::Value {
    run(source, "test.aelys").expect("Expected program to run successfully")
}

fn run_err(source: &str) -> String {
    match run(source, "test.aelys") {
        Ok(_) => panic!("Expected program to fail, but it succeeded"),
        Err(e) => format!("{}", e),
    }
}

#[test]
fn inline_interpolation_simple() {
    let result = run_ok(r#"
        let name = "world"
        "hello {name}"
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn inline_interpolation_expression() {
    let result = run_ok(r#"
        let x = 5
        "x + 1 = {x + 1}"
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn placeholder_needs_args() {
    // placeholder {} without arguments should error at compile time
    let err = run_err(r#"
        let s = "value: {}"
        s
    "#);
    assert!(err.contains("placeholder") || err.contains("argument"));
}

#[test]
fn escape_double_braces() {
    let result = run_ok(r#"
        "JSON: {{key}}"
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn mixed_literals_and_expressions() {
    let result = run_ok(r#"
        let a = 1
        let b = 2
        "a={a}, b={b}, sum={a + b}"
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn empty_format_string() {
    let result = run_ok(r#"
        ""
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn no_interpolation_fallback_to_string() {
    let result = run_ok(r#"
        "hello world"
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn tostring_builtin_exists() {
    // __tostring should be available as a builtin
    let result = run_ok(r#"
        __tostring(42)
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn tostring_converts_int() {
    let result = run_ok(r#"
        let s = __tostring(123)
        s
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn tostring_converts_float() {
    let result = run_ok(r#"
        let s = __tostring(3.14)
        s
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn tostring_converts_bool() {
    let result = run_ok(r#"
        let s = __tostring(true)
        s
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn nested_braces_in_expr() {
    // expression containing braces (like a dict literal in future)
    // for now just test that balanced braces work
    let result = run_ok(r#"
        let arr = [1, 2, 3]
        "arr = {arr}"
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn fmt_string_with_function_call() {
    let result = run_ok(r#"
        fn double(x) { x * 2 }
        "doubled: {double(5)}"
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn error_unterminated_expr() {
    let err = run_err(r#"
        "test {x"
    "#);
    assert!(err.contains("unterminated") || err.contains("}"));
}

#[test]
fn error_unmatched_close_brace() {
    let err = run_err(r#"
        "test }"
    "#);
    assert!(err.contains("unmatched") || err.contains("}"));
}

// Placeholder syntax at call site: func("fmt {}", arg)

#[test]
fn placeholder_in_call_single() {
    let result = run_ok(r#"
        fn identity(s) { s }
        identity("value: {}", 42)
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn placeholder_in_call_multiple() {
    let result = run_ok(r#"
        fn identity(s) { s }
        let x = 10
        let y = 20
        identity("x={}, y={}", x, y)
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn placeholder_in_call_with_extra_args() {
    // func("fmt {}", val, extra_arg) - extra_arg goes to func
    let result = run_ok(r#"
        fn take_two(s, n) { s }
        take_two("num: {}", 42, 99)
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn placeholder_mixed_with_inline() {
    let result = run_ok(r#"
        fn identity(s) { s }
        let name = "Reimu"
        identity("hello {name}, your number is {}", 7)
    "#);
    assert!(result.as_ptr().is_some());
}

#[test]
fn placeholder_not_enough_args() {
    let err = run_err(r#"
        fn identity(s) { s }
        identity("a={}, b={}", 1)
    "#);
    assert!(err.contains("placeholder") || err.contains("argument"));
}

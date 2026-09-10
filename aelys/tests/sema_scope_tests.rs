/// Tests for scoped signature collection and if-branch scope isolation.
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

#[allow(dead_code)]
fn sema_err(code: &str) -> bool {
    !sema_ok(code)
}

#[test]
fn toplevel_fn_not_overwritten_by_same_name_in_if() {
    assert!(
        sema_ok(
            r#"
fn compute(x: i64) -> i64 { return x + 1 }
if true {
    fn compute(x: i64) -> i64 { return x + 2 }
}
let result: i64 = compute(10)
"#
        ),
        "top-level compute() should be preserved after if block with same-name fn"
    );
}

#[test]
fn toplevel_fn_not_overwritten_by_same_name_in_while() {
    assert!(
        sema_ok(
            r#"
fn compute(x: i64) -> i64 { return x + 1 }
while false {
    fn compute(x: i64) -> i64 { return x + 2 }
}
let result: i64 = compute(10)
"#
        ),
        "top-level compute() should be preserved after while block with same-name fn"
    );
}

#[test]
fn toplevel_fn_not_overwritten_by_same_name_in_for() {
    assert!(
        sema_ok(
            r#"
fn compute(x: i64) -> i64 { return x + 1 }
for i in 0..0 {
    fn compute(x: i64) -> i64 { return x + 2 }
}
let result: i64 = compute(10)
"#
        ),
        "top-level compute() should be preserved after for block with same-name fn"
    );
}

#[test]
fn fn_in_unreachable_if_does_not_leak_to_outer_scope() {
    assert!(
        sema_ok(
            r#"
fn greet() -> i64 { return 42 }
if false {
    fn unreachable_fn() -> string { return "never runs" }
}
let x: i64 = greet()
"#
        ),
        "unreachable fn inside if-false should not affect type checking"
    );
}

#[test]
fn if_then_var_not_visible_after_if() {
    assert!(
        sema_ok(
            r#"
if true {
    let x: i64 = 42
}
let x: string = "hello"
"#
        ),
        "variable x defined in then-branch should not conflict with x after the if"
    );
}

#[test]
fn if_else_var_not_visible_after_if() {
    assert!(
        sema_ok(
            r#"
if true {
    let a: i64 = 1
} else {
    let b: string = "hi"
}
let b: i64 = 99
"#
        ),
        "variable b defined in else-branch should not conflict with b after the if"
    );
}

#[test]
fn if_then_var_not_visible_in_else() {
    assert!(
        sema_ok(
            r#"
if true {
    let val: i64 = 10
} else {
    let val: string = "ten"
}
"#
        ),
        "then-branch val:i64 should not conflict with else-branch val:string"
    );
}

#[test]
fn if_branch_scope_with_implicit_return() {
    assert!(
        sema_ok(
            r#"
fn pick(flag: bool) -> i64 {
    if flag {
        let temp: string = "computing"
        42
    } else {
        let temp: i64 = 0
        temp
    }
}
let result: i64 = pick(true)
"#
        ),
        "implicit return through if/else should have scoped branches"
    );
}

#[test]
fn literal_tracking_does_not_leak_between_functions() {
    assert!(
        sema_ok(
            r#"
fn f() {
    let x = 200
}

fn g(x: i8) -> i8 {
    return x
}
"#
        ),
        "literal tracking from one function must not pollute another function"
    );
}

#[test]
fn literal_tracking_respects_inner_scope_shadowing() {
    assert!(
        sema_ok(
            r#"
fn g(x: i8) -> i8 {
    {
        let x = 200
    }
    return x
}
"#
        ),
        "inner scoped literal shadow must not affect outer return narrowing"
    );
}

#[test]
fn mutable_shadow_must_not_make_outer_binding_assignable() {
    assert!(
        sema_err(
            r#"
fn bug() {
    let x = 1
    {
        let mut x = 2
    }
    x = 3
}
"#
        ),
        "inner let mut x must not allow assigning to outer immutable x"
    );
}

#[test]
fn function_decl_inside_block_must_not_leak_outside_block_scope() {
    assert!(
        sema_err(
            r#"
{
    fn hidden() -> i64 { return 7 }
}
fn use_it() -> i64 {
    return hidden()
}
"#
        ),
        "function declared in a block must not be callable outside that block"
    );
}

#[test]
fn block_local_function_can_shadow_outer_same_name() {
    assert!(
        sema_ok(
            r#"
fn f() -> i64 { return 1 }
{
    fn f() -> i64 { return 2 }
    let y: i64 = f()
}
let x: i64 = f()
"#
        ),
        "block-local function should be allowed to shadow outer function name"
    );
}

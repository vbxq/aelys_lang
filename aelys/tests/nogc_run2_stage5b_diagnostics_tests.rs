use aelys_driver::lower_file_to_air;
use aelys_opt::OptimizationLevel;
use std::fs;
use tempfile::tempdir;

fn reject(src: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    match lower_file_to_air(&source_path, OptimizationLevel::None) {
        Ok(_) => panic!("expected the nogc check to reject this program, but it compiled"),
        Err(err) => err.to_string(),
    }
}

fn accepts(src: &str) {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    if let Err(err) = lower_file_to_air(&source_path, OptimizationLevel::None) {
        panic!("expected this program to compile, got: {err}");
    }
}

fn assert_nogc_code(err: &str, code: &str) {
    assert!(err.contains("[nogc]"), "must carry the nogc marker: {err}");
    assert!(
        err.contains(&format!("[{code}]")),
        "must carry the {code} code: {err}"
    );
}

fn assert_anchor(err: &str, anchor: &str) {
    assert!(
        err.contains(&format!("module.aelys:{anchor}")),
        "primary caret must anchor at {anchor}, not a fallback span: {err}"
    );
}

#[test]
fn e0727_direct_intrinsic_renders_chain_and_declaration_caret() {
    let err = reject(
        r#"
use std::process::ExitCode; nogc fn f() -> i64 {
    let mut v = Vec::new();
    Vec::push(v, 1);
    0
}
fn main() -> ExitCode { return ExitCode::from(f() as u8); }
"#,
    );
    assert_nogc_code(&err, "E0727");
    assert!(
        err.contains("is declared nogc but its inferred effects reach managed memory"),
        "must be the effect-reach diagnostic: {err}"
    );
    assert!(
        err.contains("via `f -> Vec::new`"),
        "must render the call chain witness: {err}"
    );
    assert_anchor(&err, "3:17");
    assert!(
        err.contains("managed memory reached here"),
        "must show the witness caret hint: {err}"
    );
    assert!(
        err.contains("`f` is declared nogc here"),
        "must show the declaration secondary caret: {err}"
    );
}

#[test]
fn e0727_through_a_callee_names_the_hop() {
    let err = reject(
        r#"
use std::process::ExitCode; fn helper() {
    let mut v = Vec::new();
    Vec::push(v, 1);
}
nogc fn f() -> i64 {
    helper();
    0
}
fn main() -> ExitCode { return ExitCode::from(f() as u8); }
"#,
    );
    assert_nogc_code(&err, "E0727");
    assert!(
        err.contains("via `f -> helper -> Vec::new`"),
        "must render every hop of the chain: {err}"
    );
    assert_anchor(&err, "3:17");
    assert!(
        err.contains("`f` is declared nogc here"),
        "must show the declaration secondary caret: {err}"
    );
}

#[test]
fn e0728_out_of_position_renders_type_caret_and_help() {
    let err = reject(
        r#"
nogc fn my_nogc_fn() { }
fn get() -> nogc fn() { my_nogc_fn }
fn main() { }
"#,
    );
    assert_nogc_code(&err, "E0728");
    assert!(
        err.contains("only allowed as an immutable function parameter type"),
        "must be the out-of-position diagnostic: {err}"
    );
    assert_anchor(&err, "3:13");
    assert!(
        err.contains("`nogc fn` type out of position"),
        "must show the out-of-position caret hint: {err}"
    );
    assert!(
        err.contains("bare immutable parameter type"),
        "must offer the concrete alternative: {err}"
    );
}

#[test]
fn e0728_mut_param_renders_its_own_caret_and_help() {
    let err = reject(
        r#"
fn leaky(p: &i32) { }
nogc fn apply(mut f: nogc fn(&i32), x: &i32) {
    f = leaky;
    f(x);
}
fn main() { }
"#,
    );
    assert_nogc_code(&err, "E0728");
    assert!(
        err.contains("cannot be `mut`"),
        "must be the mut-param diagnostic: {err}"
    );
    assert_anchor(&err, "3:15");
    assert!(
        err.contains("`nogc fn` parameter declared `mut`"),
        "the caret hint must describe the mut reject: {err}"
    );
    assert!(
        !err.contains("`nogc fn` type out of position"),
        "the mut reject must not borrow the out-of-position caret hint: {err}"
    );
    assert!(
        err.contains("drop `mut` from `f`"),
        "must offer the concrete alternative: {err}"
    );
    // call is e0727. the help must not name an escape hatch that only trades one code for another.
    assert!(
        !err.contains("plain `fn`"),
        "the help must not suggest a plain `fn`, which an enclosing nogc fn rejects: {err}"
    );
}

#[test]
fn e0728_mut_help_advice_actually_compiles() {
    accepts(
        r#"
nogc fn ok(p: &i32) { }
nogc fn apply(f: nogc fn(&i32), x: &i32) { f(x); }
fn main() {
    let n: i32 = 5;
    apply(ok, &n);
}
"#,
    );
    let err = reject(
        r#"
fn leaky(p: &i32) { }
nogc fn apply(mut f: fn(&i32), x: &i32) {
    f = leaky;
    f(x);
}
fn main() { }
"#,
    );
    assert!(
        err.contains("[E0727]"),
        "the replaced advice trades E0728 for E0727, which is why it is gone: {err}"
    );
}

#[test]
fn e0728_param_shadow_renders_its_own_caret_and_help() {
    let err = reject(
        r#"
nogc fn ok(p: &i32) { }
fn leaky(p: &i32) { }
nogc fn apply(f: nogc fn(&i32), x: &i32) { f(x); }
nogc fn outer(f: nogc fn(&i32), x: &i32) {
    let mut f = ok;
    f = leaky;
    apply(f, x);
}
fn main() { }
"#,
    );
    assert_nogc_code(&err, "E0728");
    assert!(
        err.contains("cannot be shadowed by a `let` binding"),
        "must be the shadow diagnostic: {err}"
    );
    assert_anchor(&err, "6:5");
    assert!(
        err.contains("shadows a `nogc fn` parameter"),
        "the caret hint must describe the shadow reject: {err}"
    );
    assert!(
        !err.contains("`nogc fn` type out of position"),
        "the shadow reject must not borrow the out-of-position caret hint: {err}"
    );
    assert!(
        err.contains("give the binding a different name"),
        "must offer the concrete alternative: {err}"
    );
}

#[test]
fn e0729_general_fn_argument_anchors_on_the_argument() {
    let err = reject(
        r#"
fn bad(p: &i32) { }
nogc fn apply(f: nogc fn(&i32), x: &i32) { f(x); }
fn main() {
    let n: i32 = 5;
    apply(bad, &n);
}
"#,
    );
    assert_nogc_code(&err, "E0729");
    assert!(
        err.contains("expected a `nogc fn` argument"),
        "must be the callback-mismatch diagnostic: {err}"
    );
    assert_anchor(&err, "6:11");
    assert!(
        err.contains("not a `nogc fn`"),
        "must show the argument caret hint: {err}"
    );
    assert!(
        err.contains("pass a `nogc`-declared function by name"),
        "must offer the concrete alternative: {err}"
    );
}

#[test]
fn e0729_lambda_argument_anchors_on_the_lambda() {
    let err = reject(
        r#"
nogc fn apply(f: nogc fn(&i32), x: &i32) { f(x); }
fn main() {
    let n: i32 = 5;
    apply(move |p: &i32| { }, &n);
}
"#,
    );
    assert_nogc_code(&err, "E0729");
    assert_anchor(&err, "5:11");
    assert!(
        err.contains("not a `nogc fn`"),
        "must show the argument caret hint: {err}"
    );
}

// the unresolved case has no culprit argument, so the callee stays the primary and no secondary
#[test]
fn e0731_anchors_on_the_value_reference() {
    let err = reject(
        r#"
nogc fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() {
    let g: fn(i64) -> i64 = keep;
}
"#,
    );
    assert_nogc_code(&err, "E0731");
    assert!(
        err.contains("may only be called directly, never used as a value"),
        "must be the generic-as-value diagnostic: {err}"
    );
    assert_anchor(&err, "4:29");
    assert!(
        err.contains("`nogc` generic used as a value"),
        "must show the value-reference caret hint: {err}"
    );
    assert!(
        err.contains("call it directly"),
        "must offer the concrete alternative: {err}"
    );
}

#[test]
fn explain_e0728_covers_all_three_shapes() {
    let info = aelys_common::registry::lookup("E0728").expect("E0728 must be registered");
    let text = format!("{} {}", info.title, info.explanation);
    assert!(
        text.contains("return type") && text.contains("let binding"),
        "must describe the out-of-position shape: {text}"
    );
    assert!(
        text.contains("`mut`"),
        "must describe the mut-parameter shape: {text}"
    );
    assert!(
        text.contains("shadow"),
        "must describe the let-shadow shape: {text}"
    );
}

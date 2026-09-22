use aelys_driver::lower_file_to_air;
use aelys_opt::OptimizationLevel;
use std::fs;
use tempfile::tempdir;

fn lower(src: &str) -> Result<(), String> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    lower_file_to_air(&source_path, OptimizationLevel::None)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn reject(src: &str) -> String {
    match lower(src) {
        Ok(()) => panic!("expected the nogc check to reject this program, but it compiled"),
        Err(err) => err,
    }
}

fn accepts(src: &str) {
    lower(src).unwrap_or_else(|err| panic!("the nogc check must accept this program: {err}"));
}

fn assert_code(err: &str, code: &str) {
    assert!(err.contains(code), "must carry the {code} code: {err}");
}

#[test]
fn compile_nogc_to_general_let() {
    accepts(
        "\
nogc fn my_nogc_fn() { }
fn main() -> i64 {
    let g: fn() = my_nogc_fn
    return 0
}
",
    );
}

#[test]
fn compile_higher_order_apply() {
    accepts(
        "\
nogc fn my_nogc_fn(p: &i32) { }
nogc fn apply(f: nogc fn(&i32), x: &i32) { f(x) }
fn main() -> i64 {
    let n: i32 = 5
    apply(my_nogc_fn, &n)
    return 0
}
",
    );
}

#[test]
fn compile_forward_nogc_param() {
    accepts(
        "\
nogc fn my_nogc_fn(p: &i32) { }
nogc fn apply(f: nogc fn(&i32), x: &i32) { f(x) }
nogc fn outer(g: nogc fn(&i32), y: &i32) { apply(g, y) }
fn main() -> i64 {
    let n: i32 = 5
    outer(my_nogc_fn, &n)
    return 0
}
",
    );
}

#[test]
fn reject_general_fn_arg() {
    let err = reject(
        "\
fn bad(p: &i32) { }
nogc fn apply(f: nogc fn(&i32), x: &i32) { f(x) }
fn main() -> i64 {
    let n: i32 = 5
    apply(bad, &n)
    return 0
}
",
    );
    assert_code(&err, "E0729");
}

#[test]
fn reject_lambda_arg() {
    let err = reject(
        "\
nogc fn apply(f: nogc fn(&i32), x: &i32) { f(x) }
fn main() -> i64 {
    let n: i32 = 5
    apply(fn(p: &i32) { }, &n)
    return 0
}
",
    );
    assert_code(&err, "E0729");
}

// a let-bound variable is rejected even when it shadows a nogc-fn name. scope-aware,
#[test]
fn reject_let_var_arg_shadowing() {
    let err = reject(
        "\
nogc fn ok(p: &i32) { }
fn leaky(p: &i32) { }
nogc fn apply(f: nogc fn(&i32), x: &i32) { f(x) }
fn main() -> i64 {
    let n: i32 = 5
    let ok = leaky
    apply(ok, &n)
    return 0
}
",
    );
    assert_code(&err, "E0729");
}

// immutable param with an inferred binding. `let mut f = ok` infers `f: nogc fn`, then `f = leaky`
// type -> wrongly accept, running `leaky` (allocs a vec) inside the certified-nogc `apply`.
// the shadow is now rejected at the let site , so the param name always resolves to the
// sema/src/infer/stmt/let_stmt.rs::infer_let_stmt -> both witnesses below compile (the leak).
#[test]
fn reject_let_shadows_nogc_param() {
    let err = reject(
        "\
nogc fn ok(p: &i32) { }
fn leaky(p: &i32) {
    let mut v = Vec::new()
    Vec::push(v, 1)
}
nogc fn apply(f: nogc fn(&i32), x: &i32) { f(x) }
nogc fn outer(f: nogc fn(&i32), x: &i32) {
    let mut f = ok
    f = leaky
    apply(f, x)
}
fn main() -> i64 { return 0 }
",
    );
    assert_code(&err, "E0728");
}

#[test]
fn reject_let_shadows_nogc_param_two_hop() {
    let err = reject(
        "\
nogc fn ok(p: &i32) { }
fn leaky(p: &i32) {
    let mut v = Vec::new()
    Vec::push(v, 1)
}
nogc fn apply(f: nogc fn(&i32), x: &i32) { f(x) }
nogc fn mid(f: nogc fn(&i32), x: &i32) { apply(f, x) }
nogc fn outer(f: nogc fn(&i32), x: &i32) {
    let mut f = ok
    f = leaky
    mid(f, x)
}
fn main() -> i64 { return 0 }
",
    );
    assert_code(&err, "E0728");
}

// frozen param type still tightens `f(x)`, leaking managed effects. rejected at param lowering.
#[test]
fn reject_fcrit_mut_nogc_param() {
    let err = reject(
        "\
fn leaky(p: &i32) { }
nogc fn apply(mut f: nogc fn(&i32), x: &i32) {
    f = leaky
    f(x)
}
fn main() -> i64 { return 0 }
",
    );
    assert_code(&err, "E0728");
}

// param forwarded onward escapes a tightening-site-only patch. the mut-reject closes it too.
#[test]
fn reject_fcrit_mut_forwarding() {
    let err = reject(
        "\
fn leaky(p: &i32) { }
nogc fn apply2(g: nogc fn(&i32), y: &i32) { g(y) }
nogc fn apply(mut f: nogc fn(&i32), x: &i32) {
    f = leaky
    apply2(f, x)
}
fn main() -> i64 { return 0 }
",
    );
    assert_code(&err, "E0728");
}

// a nogc-fn type in a let-binding annotation is out of position.
#[test]
fn reject_nogc_in_let_annotation() {
    let err = reject(
        "\
nogc fn my_nogc_fn() { }
fn main() -> i64 {
    let g: nogc fn() = my_nogc_fn
    return 0
}
",
    );
    assert_code(&err, "E0728");
}

// a nogc-fn type in a return-type annotation is out of position.
#[test]
fn reject_nogc_in_return_type() {
    let err = reject(
        "\
nogc fn my_nogc_fn() { }
fn get() -> nogc fn() { return my_nogc_fn }
fn main() -> i64 { return 0 }
",
    );
    assert_code(&err, "E0728");
}

// a nogc-fn type as an aggregate element (here an array element) is out of position.
#[test]
fn reject_nogc_in_aggregate() {
    let err = reject(
        "\
fn take(xs: [nogc fn(); 2]) { }
fn main() -> i64 { return 0 }
",
    );
    assert_code(&err, "E0728");
}

// a nogc-fn type nested inside another fn type (even a parameter one) is out of position.
#[test]
fn reject_nogc_nested_fn_type() {
    let err = reject(
        "\
nogc fn apply(f: nogc fn(nogc fn())) { }
fn main() -> i64 { return 0 }
",
    );
    assert_code(&err, "E0728");
}

// ============================ must-reject: the two killed leaks (v6 closes them) ============================

#[test]
fn reject_inferred_aggregate_index_call() {
    let err = reject(
        "\
nogc fn ok() { }
fn bad() { }
nogc fn f() -> i64 {
    let arr = [ok, bad]
    arr[1]()
    return 0
}
fn main() -> i64 { return f() }
",
    );
    // an inferred aggregate cannot carry the nogc marker, so the indirect call is not tightened
    assert_code(&err, "E0727");
}

#[test]
fn reject_if_merge_indirect_call() {
    let err = reject(
        "\
nogc fn ok() { }
fn bad() { }
nogc fn f() -> i64 {
    let c = true
    let h = if c { ok } else { bad }
    h()
    return 0
}
fn main() -> i64 { return f() }
",
    );
    assert_code(&err, "E0727");
}

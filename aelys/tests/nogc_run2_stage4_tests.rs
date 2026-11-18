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
        Ok(()) => panic!("expected the nogc bound check to reject this program, but it compiled"),
        Err(err) => err,
    }
}

fn accepts(src: &str) {
    lower(src).unwrap_or_else(|err| panic!("the nogc bound check must accept this program: {err}"));
}

fn assert_code(err: &str, code: &str) {
    assert!(
        err.contains(&format!("[{code}]")),
        "must carry the {code} code: {err}"
    );
    assert!(err.contains("[nogc]"), "must carry the nogc marker: {err}");
}

// the non-generic half was already rejected at head; it anchors the pair.
#[test]
fn reject_witness_non_generic_vec_by_value_is_still_e0727() {
    let err = reject(
        "\
use std::process::ExitCode; nogc fn keep(x: vec<i64>) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let v = vec![1, 2]; return ExitCode::from(keep(v) as u8); }
",
    );
    assert_code(&err, "E0727");
}

#[test]
fn reject_witness_generic_vec_by_value_is_now_e0730() {
    let err = reject(
        "\
use std::process::ExitCode; nogc fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let v = vec![1, 2]; return ExitCode::from(keep(v) as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("`T` of `keep`") && err.contains("Vec<i64>"),
        "must name the type param and the offending type: {err}"
    );
}

#[test]
fn compile_witness_generic_instantiated_with_i64() {
    accepts(
        "\
use std::process::ExitCode; nogc fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { return ExitCode::from(keep(7) as u8); }
",
    );
}

#[test]
fn plain_generic_with_a_vec_argument_is_now_e0412() {
    let err = reject(
        "\
use std::process::ExitCode; fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let v = vec![1, 2]; return ExitCode::from(keep(v) as u8); }
",
    );
    assert!(
        err.contains("[E0412]"),
        "a vec passed to a generic is out of the Vec surface: {err}"
    );
}

// implicit bounds on a nogc-declared generic ====================

#[test]
fn reject_implicit_bound_instantiated_with_rc() {
    let err = reject(
        "\
use std::process::ExitCode; nogc fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let r = Rc::new(7); return ExitCode::from(keep(r) as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(err.contains("Rc<i64>"), "must name the Rc type: {err}");
}

#[test]
fn compile_twin_plain_generic_instantiated_with_rc() {
    accepts(
        "\
use std::process::ExitCode; fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let r = Rc::new(7); return ExitCode::from(keep(r) as u8); }
",
    );
}

#[test]
fn reject_implicit_bound_instantiated_with_string() {
    let err = reject(
        "\
use std::process::ExitCode; nogc fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let s = \"hi\"; return ExitCode::from(keep(s) as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(err.contains("string"), "must name the string type: {err}");
}

#[test]
fn compile_twin_plain_generic_instantiated_with_string() {
    accepts(
        "\
use std::process::ExitCode; fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let s = \"hi\"; return ExitCode::from(keep(s) as u8); }
",
    );
}

// a nogc-bound type param in return-only position can never be pinned by an argument, so it is a
#[test]
fn reject_bound_type_param_never_pinned_by_an_argument() {
    let err = reject(
        "\
use std::process::ExitCode; nogc fn make<U: nogc>() -> i64 { 0 }
fn main() -> ExitCode { return ExitCode::from(make() as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("does not pin it to a concrete type"),
        "must be the fail-closed unresolved-binding message: {err}"
    );
}

#[test]
fn compile_twin_bound_type_param_pinned_by_an_argument() {
    accepts(
        "\
use std::process::ExitCode; nogc fn make<U: nogc>(u: U) -> i64 { 0 }
fn main() -> ExitCode { return ExitCode::from(make(1) as u8); }
",
    );
}

#[test]
fn reject_explicit_bound_instantiated_with_vec() {
    let err = reject(
        "\
use std::process::ExitCode; fn keep<T: nogc>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let v = vec![1, 2]; return ExitCode::from(keep(v) as u8); }
",
    );
    assert_code(&err, "E0730");
}

#[test]
fn compile_twin_explicit_bound_instantiated_with_i64() {
    accepts(
        "\
use std::process::ExitCode; fn keep<T: nogc>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { return ExitCode::from(keep(3) as u8); }
",
    );
}

#[test]
fn reject_explicit_bound_instantiated_with_rc() {
    let err = reject(
        "\
use std::process::ExitCode; fn keep<T: nogc>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let r = Rc::new(7); return ExitCode::from(keep(r) as u8); }
",
    );
    assert_code(&err, "E0730");
}

#[test]
fn reject_mixed_bounds_only_the_bound_param_fires() {
    let err = reject(
        "\
use std::process::ExitCode; fn pick<T: nogc, U>(a: T, b: U) -> i64 { let x = a; let y = b; 0 }
fn main() -> ExitCode { let v = vec![1, 2]; return ExitCode::from(pick(v, 3) as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("`T` of `pick`"),
        "must name `T`, not `U`: {err}"
    );
}

// retired by stage 1 increment 1a (was compile_twin_mixed_bounds_vec_in_the_unbounded_slot, an
// `vec` in any generic slot is now e0412 regardless of nogc bounds, so a vec in the unbounded slot
// can no longer show that only the bound param fires. it guarded reject_mixed_bounds_only_the_bound_param_fires.
#[test]
fn mixed_bounds_vec_in_the_unbounded_slot_is_now_e0412() {
    let err = reject(
        "\
use std::process::ExitCode; fn pick<T: nogc, U>(a: T, b: U) -> i64 { let x = a; let y = b; 0 }
fn main() -> ExitCode { let v = vec![1, 2]; return ExitCode::from(pick(3, v) as u8); }
",
    );
    assert!(
        err.contains("[E0412]"),
        "a vec passed to an unbounded generic slot is out of the Vec surface: {err}"
    );
}

// an abstract type param cannot satisfy the bound ====================

#[test]
fn reject_abstract_type_param_fed_to_a_bound_param() {
    let err = reject(
        "\
use std::process::ExitCode; nogc fn sink<U: nogc>(u: U) -> i64 { 0 }
fn forward<T>(x: T) -> i64 { sink(x) }
fn main() -> ExitCode { return ExitCode::from(forward(1) as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("`U` of `sink`") && err.contains("with `T`"),
        "must point at the unproven abstract param: {err}"
    );
}

#[test]
fn compile_twin_same_forwarding_to_an_unbounded_target() {
    accepts(
        "\
use std::process::ExitCode; fn sink<U>(u: U) -> i64 { 0 }
fn forward<T>(x: T) -> i64 { sink(x) }
fn main() -> ExitCode { return ExitCode::from(forward(1) as u8); }
",
    );
}

#[test]
fn reject_struct_hiding_an_rc_behind_a_nominal() {
    let err = reject(
        "\
use std::process::ExitCode; struct Node { v: i64 }
struct Holder { n: Rc<Node> }
nogc fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let h = Holder { n: Rc::new(Node { v: 1 }) }; return ExitCode::from(keep(h) as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(err.contains("`Holder`"), "must name the struct: {err}");
}

#[test]
fn compile_twin_same_struct_through_an_unbounded_generic() {
    accepts(
        "\
use std::process::ExitCode; struct Node { v: i64 }
struct Holder { n: Rc<Node> }
fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let h = Holder { n: Rc::new(Node { v: 1 }) }; return ExitCode::from(keep(h) as u8); }
",
    );
}

#[test]
fn reject_struct_hiding_a_string_field() {
    let err = reject(
        "\
use std::process::ExitCode; struct Msg { text: String }
nogc fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let m = Msg { text: \"hi\" }; return ExitCode::from(keep(m) as u8); }
",
    );
    assert_code(&err, "E0730");
}

#[test]
fn generic_struct_argument_binds_the_nogc_type_param_to_its_type_argument() {
    accepts(
        "\
use std::process::ExitCode; struct Box2<T> { v: T }
nogc fn keep<U>(x: Box2<U>) -> i64 { 0 }
fn main() -> ExitCode { let b = Box2 { v: 1 }; return ExitCode::from(keep(b) as u8); }
",
    );
    let err = reject(
        "\
use std::process::ExitCode; struct Box2<T> { v: T }
nogc fn keep<U>(x: Box2<U>) -> i64 { 0 }
fn main() -> ExitCode { let b = Box2 { v: \"s\" + \"t\" }; return ExitCode::from(keep(b) as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("`U` of `keep`") && err.contains("string"),
        "must name the type param and the offending type: {err}"
    );
}

#[test]
fn compile_twin_all_primitive_value_struct() {
    accepts(
        "\
use std::process::ExitCode; struct P { x: i64, y: i64 }
nogc fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let p = P { x: 1, y: 2 }; return ExitCode::from(keep(p) as u8); }
",
    );
}

#[test]
fn reject_enum_hiding_a_string_payload() {
    let err = reject(
        "\
use std::process::ExitCode; enum Bad { None, Some(String) }
nogc fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let o = Bad::Some(\"hi\"); return ExitCode::from(keep(o) as u8); }
",
    );
    assert_code(&err, "E0730");
}

#[test]
fn compile_twin_enum_with_only_nogc_payloads() {
    accepts(
        "\
use std::process::ExitCode; enum Opt { None, Some(i64) }
nogc fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let o = Opt::Some(3); return ExitCode::from(keep(o) as u8); }
",
    );
}

#[test]
fn compile_fixed_array_is_a_nogc_value() {
    accepts(
        "\
use std::process::ExitCode; nogc fn keep<T>(v: T) -> i64 { let y = v; 0 }
fn main() -> ExitCode { let a = [1, 2, 3]; return ExitCode::from(keep(a) as u8); }
",
    );
}

#[test]
fn reject_nogc_generic_bound_to_an_annotated_let() {
    let err = reject(
        "\
use std::process::ExitCode; nogc fn sink<U>(u: U) -> i64 { 0 }
fn main() -> ExitCode { let v = vec![1, 2]; let f: fn(vec<i64>) -> i64 = sink; return ExitCode::from(f(v) as u8); }
",
    );
    assert_code(&err, "E0731");
    assert!(err.contains("`sink`"), "must name the generic: {err}");
}

#[test]
fn twin_plain_generic_bound_to_an_annotated_let_is_refused_without_the_nogc_code() {
    let err = lower(
        "\
use std::process::ExitCode; fn sink<U>(u: U) -> i64 { 0 }
fn main() -> ExitCode { let v = vec![1, 2]; let f: fn(vec<i64>) -> i64 = sink; return ExitCode::from(f(v) as u8); }
",
    )
    .expect_err("a generic function used as a value is refused");
    assert!(
        err.contains("[E0902]") && err.contains("a generic function cannot be used as a value yet"),
        "must carry the generic-value E0902: {err}"
    );
    assert!(
        !err.contains("E0731"),
        "the nogc bound code must not fire: {err}"
    );
}

#[test]
fn reject_nogc_generic_passed_as_an_argument() {
    let err = reject(
        "\
use std::process::ExitCode; fn keep<T: nogc>(x: i64) -> i64 { x }
fn take(f: fn(i64) -> i64) -> i64 { f(1) }
fn main() -> ExitCode { return ExitCode::from(take(keep) as u8); }
",
    );
    assert_code(&err, "E0731");
    assert!(
        !err.contains("E0301") && !err.contains("E0304"),
        "E0731 must be the only error, so the test isolates R5: {err}"
    );
}

#[test]
fn compile_twin_non_generic_nogc_fn_passed_as_a_nogc_argument() {
    accepts(
        "\
use std::process::ExitCode; nogc fn one() -> i64 { 1 }
fn apply(f: nogc fn() -> i64) -> i64 { f() }
fn main() -> ExitCode { return ExitCode::from(apply(one) as u8); }
",
    );
}

#[test]
fn compile_unbounded_generic_is_unchanged() {
    accepts(
        "\
use std::process::ExitCode; fn ident<T>(x: T) -> i64 { 0 }
fn main() -> ExitCode { return ExitCode::from(ident(41) as u8); }
",
    );
}

// so the matcher must refuse to bind it; fail-closed then rejects rather than reading the struct.
#[test]
fn reject_bound_type_param_name_shadowed_by_a_real_struct() {
    let err = reject(
        "\
use std::process::ExitCode; struct T { a: i64 }
nogc fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let t = T { a: 1 }; return ExitCode::from(keep(t) as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("does not pin it to a concrete type"),
        "the matcher must not bind a name that is a real struct: {err}"
    );
}

#[test]
fn compile_twin_shadowed_type_param_name_without_nogc() {
    accepts(
        "\
use std::process::ExitCode; struct T { a: i64 }
fn keep<T>(x: T) -> i64 { let y = x; 0 }
fn main() -> ExitCode { let t = T { a: 1 }; return ExitCode::from(keep(t) as u8); }
",
    );
}

#[test]
fn reject_nested_nogc_generic_instantiated_with_vec() {
    let err = reject(
        "\
use std::process::ExitCode; fn outer() -> i64 {
    nogc fn helper<T>(x: T) -> i64 { let y = x; 0 }
    let v = vec![1, 2];
    helper(v)
}
fn main() -> ExitCode { return ExitCode::from(outer() as u8); }
",
    );
    assert_code(&err, "E0730");
}

#[test]
fn compile_twin_nested_nogc_generic_instantiated_with_i64() {
    accepts(
        "\
use std::process::ExitCode; fn outer() -> i64 {
    nogc fn helper<T>(x: T) -> i64 { let y = x; 0 }
    helper(1)
}
fn main() -> ExitCode { return ExitCode::from(outer() as u8); }
",
    );
}

// check has no scope state, so a function shadowing a bound generic is checked against the bound
// it shadows. that is fail-closed (a spurious reject), never fail-open, and it is pinned here so
#[test]
fn reject_unbounded_generic_shadowing_a_bound_name_is_fail_closed() {
    let err = reject(
        "\
use std::process::ExitCode; nogc fn helper<T>(x: T) -> i64 { 0 }
fn outer() -> i64 {
    fn helper<U>(y: U) -> i64 { 1 }
    let v = vec![1, 2];
    helper(v)
}
fn main() -> ExitCode { return ExitCode::from(outer() as u8); }
",
    );
    assert_code(&err, "E0730");
}

#[test]
fn reject_partially_bound_shadow_cannot_weaken_a_bound_name() {
    let err = reject(
        "\
use std::process::ExitCode; nogc fn h<A, B>(a: A, b: B) -> i64 { let x = a; 0 }
fn outer() -> i64 {
    fn h<A, B: nogc>(a: A, b: B) -> i64 { 0 }
    0
}
fn main() -> ExitCode { let v = vec![1, 2]; return ExitCode::from((h(v, 1) + outer()) as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("`A` of `h`") && err.contains("Vec<i64>"),
        "the bound on the FIRST type param must survive the shadow: {err}"
    );
}

#[test]
fn reject_shadow_with_different_type_param_names_cannot_weaken() {
    let err = reject(
        "\
use std::process::ExitCode; nogc fn h<A, B>(a: A, b: B) -> i64 { let x = a; 0 }
fn outer() -> i64 {
    nogc fn h<B>(b: B) -> i64 { 0 }
    0
}
fn main() -> ExitCode { let v = vec![1, 2]; return ExitCode::from((h(v, 1) + outer()) as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("`A` of `h`") && err.contains("Vec<i64>"),
        "the wider signature's bound must survive the narrower shadow: {err}"
    );
}

#[test]
fn reject_sibling_nested_generics_sharing_a_bare_name() {
    let err = reject(
        "\
use std::process::ExitCode; fn a() -> i64 {
    nogc fn h<A, B>(x: A, y: B) -> i64 { let z = x; 0 }
    let v = vec![1, 2];
    h(v, 1)
}
fn b() -> i64 {
    fn h<A, B: nogc>(x: A, y: B) -> i64 { 0 }
    0
}
fn main() -> ExitCode { return ExitCode::from((a() + b()) as u8); }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("`A` of `h`") && err.contains("Vec<i64>"),
        "a sibling scope's weaker signature must not erase the bound: {err}"
    );
}

#[test]
fn bare_name_collision_without_any_nogc_is_now_e0418() {
    let err = reject(
        "\
use std::process::ExitCode; fn h<A, B>(a: A, b: B) -> i64 { let x = a; 0 }
fn outer() -> i64 {
    fn h<A, B>(a: A, b: B) -> i64 { 0 }
    0
}
fn main() -> ExitCode { let v = vec![1, 2]; return ExitCode::from((h(v, 1) + outer()) as u8); }
",
    );
    assert!(
        err.contains("[E0418]"),
        "a nested fn reusing a top-level name is rejected at its root: {err}"
    );
}

#[test]
fn generic_shadowing_with_a_vec_argument_is_now_e0418() {
    let err = reject(
        "\
use std::process::ExitCode; fn helper<T>(x: T) -> i64 { 0 }
fn outer() -> i64 {
    fn helper<U>(y: U) -> i64 { 1 }
    let v = vec![1, 2];
    helper(v)
}
fn main() -> ExitCode { return ExitCode::from(outer() as u8); }
",
    );
    assert!(
        err.contains("[E0418]"),
        "a nested fn shadowing a top-level generic is rejected at its root: {err}"
    );
}

// a nogc-declared non-generic function has no type params, so the side table stays empty for it.
#[test]
fn compile_non_generic_nogc_function_is_unaffected() {
    accepts(
        "\
use std::process::ExitCode; nogc fn add(a: i64, b: i64) -> i64 { a + b }
fn main() -> ExitCode { return ExitCode::from(add(1, 2) as u8); }
",
    );
}

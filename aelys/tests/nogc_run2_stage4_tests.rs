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
nogc fn keep(x: vec<i64>) -> i64 { let y = x; return 0 }
fn main() -> i64 { let v = vec[1, 2]; return keep(v) }
",
    );
    assert_code(&err, "E0727");
}

#[test]
fn reject_witness_generic_vec_by_value_is_now_e0730() {
    let err = reject(
        "\
nogc fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let v = vec[1, 2]; return keep(v) }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("`T` of `keep`") && err.contains("vec[i64]"),
        "must name the type param and the offending type: {err}"
    );
}

#[test]
fn compile_witness_generic_instantiated_with_i64() {
    accepts(
        "\
nogc fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { return keep(7) }
",
    );
}

#[test]
fn plain_generic_with_a_vec_argument_is_now_e0412() {
    let err = reject(
        "\
fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let v = vec[1, 2]; return keep(v) }
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
nogc fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let r = Rc::new(7); return keep(r) }
",
    );
    assert_code(&err, "E0730");
    assert!(err.contains("Rc<i64>"), "must name the Rc type: {err}");
}

#[test]
fn compile_twin_plain_generic_instantiated_with_rc() {
    accepts(
        "\
fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let r = Rc::new(7); return keep(r) }
",
    );
}

#[test]
fn reject_implicit_bound_instantiated_with_string() {
    let err = reject(
        "\
nogc fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let s = \"hi\"; return keep(s) }
",
    );
    assert_code(&err, "E0730");
    assert!(err.contains("string"), "must name the string type: {err}");
}

#[test]
fn compile_twin_plain_generic_instantiated_with_string() {
    accepts(
        "\
fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let s = \"hi\"; return keep(s) }
",
    );
}

// a nogc-bound type param in return-only position can never be pinned by an argument, so it is a
#[test]
fn reject_bound_type_param_never_pinned_by_an_argument() {
    let err = reject(
        "\
nogc fn make<U: nogc>() -> i64 { return 0 }
fn main() -> i64 { return make() }
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
nogc fn make<U: nogc>(u: U) -> i64 { return 0 }
fn main() -> i64 { return make(1) }
",
    );
}

#[test]
fn reject_explicit_bound_instantiated_with_vec() {
    let err = reject(
        "\
fn keep<T: nogc>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let v = vec[1, 2]; return keep(v) }
",
    );
    assert_code(&err, "E0730");
}

#[test]
fn compile_twin_explicit_bound_instantiated_with_i64() {
    accepts(
        "\
fn keep<T: nogc>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { return keep(3) }
",
    );
}

#[test]
fn reject_explicit_bound_instantiated_with_rc() {
    let err = reject(
        "\
fn keep<T: nogc>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let r = Rc::new(7); return keep(r) }
",
    );
    assert_code(&err, "E0730");
}

#[test]
fn reject_mixed_bounds_only_the_bound_param_fires() {
    let err = reject(
        "\
fn pick<T: nogc, U>(a: T, b: U) -> i64 { let x = a; let y = b; return 0 }
fn main() -> i64 { let v = vec[1, 2]; return pick(v, 3) }
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
fn pick<T: nogc, U>(a: T, b: U) -> i64 { let x = a; let y = b; return 0 }
fn main() -> i64 { let v = vec[1, 2]; return pick(3, v) }
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
nogc fn sink<U: nogc>(u: U) -> i64 { return 0 }
fn forward<T>(x: T) -> i64 { return sink(x) }
fn main() -> i64 { return forward(1) }
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
fn sink<U>(u: U) -> i64 { return 0 }
fn forward<T>(x: T) -> i64 { return sink(x) }
fn main() -> i64 { return forward(1) }
",
    );
}

#[test]
fn reject_struct_hiding_an_rc_behind_a_nominal() {
    let err = reject(
        "\
struct Node { v: i64 }
struct Holder { n: Rc<Node> }
nogc fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let h = Holder { n: Rc::new(Node { v: 1 }) }; return keep(h) }
",
    );
    assert_code(&err, "E0730");
    assert!(err.contains("`Holder`"), "must name the struct: {err}");
}

#[test]
fn compile_twin_same_struct_through_an_unbounded_generic() {
    accepts(
        "\
struct Node { v: i64 }
struct Holder { n: Rc<Node> }
fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let h = Holder { n: Rc::new(Node { v: 1 }) }; return keep(h) }
",
    );
}

#[test]
fn reject_struct_hiding_a_string_field() {
    let err = reject(
        "\
struct Msg { text: string }
nogc fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let m = Msg { text: \"hi\" }; return keep(m) }
",
    );
    assert_code(&err, "E0730");
}

// a generic struct argument cannot be pinned and cannot be walked, so the message must name that
// rather than ask for a concrete type the user has no way to supply.
#[test]
fn reject_generic_struct_argument_names_the_real_reason() {
    let err = reject(
        "\
struct Box2<T> { v: T }
nogc fn keep<U>(x: Box2<U>) -> i64 { return 0 }
fn main() -> i64 { let b = Box2 { v: 1 }; return keep(b) }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("generic struct `Box2`") && !err.contains("does not pin it"),
        "must blame the generic struct, not the call's argument list: {err}"
    );
}

#[test]
fn compile_twin_all_primitive_value_struct() {
    accepts(
        "\
struct P { x: i64, y: i64 }
nogc fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let p = P { x: 1, y: 2 }; return keep(p) }
",
    );
}

#[test]
fn reject_enum_hiding_a_string_payload() {
    let err = reject(
        "\
enum Bad { None, Some(string) }
nogc fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let o = Bad::Some(\"hi\"); return keep(o) }
",
    );
    assert_code(&err, "E0730");
}

#[test]
fn compile_twin_enum_with_only_nogc_payloads() {
    accepts(
        "\
enum Opt { None, Some(i64) }
nogc fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let o = Opt::Some(3); return keep(o) }
",
    );
}

#[test]
fn compile_fixed_array_is_a_nogc_value() {
    accepts(
        "\
nogc fn keep<T>(v: T) -> i64 { let y = v; return 0 }
fn main() -> i64 { let a = [1, 2, 3]; return keep(a) }
",
    );
}

#[test]
fn reject_nogc_generic_bound_to_an_annotated_let() {
    let err = reject(
        "\
nogc fn sink<U>(u: U) -> i64 { return 0 }
fn main() -> i64 { let v = vec[1, 2]; let f: fn(vec<i64>) -> i64 = sink; return f(v) }
",
    );
    assert_code(&err, "E0731");
    assert!(err.contains("`sink`"), "must name the generic: {err}");
}

#[test]
fn compile_twin_plain_generic_bound_to_an_annotated_let() {
    accepts(
        "\
fn sink<U>(u: U) -> i64 { return 0 }
fn main() -> i64 { let v = vec[1, 2]; let f: fn(vec<i64>) -> i64 = sink; return f(v) }
",
    );
}

#[test]
fn reject_nogc_generic_passed_as_an_argument() {
    let err = reject(
        "\
fn keep<T: nogc>(x: i64) -> i64 { return x }
fn take(f: fn(i64) -> i64) -> i64 { return f(1) }
fn main() -> i64 { return take(keep) }
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
nogc fn one() -> i64 { return 1 }
fn apply(f: nogc fn() -> i64) -> i64 { return f() }
fn main() -> i64 { return apply(one) }
",
    );
}

#[test]
fn compile_unbounded_generic_is_unchanged() {
    accepts(
        "\
fn ident<T>(x: T) -> i64 { return 0 }
fn main() -> i64 { return ident(41) }
",
    );
}

// so the matcher must refuse to bind it; fail-closed then rejects rather than reading the struct.
#[test]
fn reject_bound_type_param_name_shadowed_by_a_real_struct() {
    let err = reject(
        "\
struct T { a: i64 }
nogc fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let t = T { a: 1 }; return keep(t) }
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
struct T { a: i64 }
fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 { let t = T { a: 1 }; return keep(t) }
",
    );
}

#[test]
fn reject_nested_nogc_generic_instantiated_with_vec() {
    let err = reject(
        "\
fn outer() -> i64 {
    nogc fn helper<T>(x: T) -> i64 { let y = x; return 0 }
    let v = vec[1, 2]
    return helper(v)
}
fn main() -> i64 { return outer() }
",
    );
    assert_code(&err, "E0730");
}

#[test]
fn compile_twin_nested_nogc_generic_instantiated_with_i64() {
    accepts(
        "\
fn outer() -> i64 {
    nogc fn helper<T>(x: T) -> i64 { let y = x; return 0 }
    return helper(1)
}
fn main() -> i64 { return outer() }
",
    );
}

// check has no scope state, so a function shadowing a bound generic is checked against the bound
// it shadows. that is fail-closed (a spurious reject), never fail-open, and it is pinned here so
#[test]
fn reject_unbounded_generic_shadowing_a_bound_name_is_fail_closed() {
    let err = reject(
        "\
nogc fn helper<T>(x: T) -> i64 { return 0 }
fn outer() -> i64 {
    fn helper<U>(y: U) -> i64 { return 1 }
    let v = vec[1, 2]
    return helper(v)
}
fn main() -> i64 { return outer() }
",
    );
    assert_code(&err, "E0730");
}

#[test]
fn reject_partially_bound_shadow_cannot_weaken_a_bound_name() {
    let err = reject(
        "\
nogc fn h<A, B>(a: A, b: B) -> i64 { let x = a; return 0 }
fn outer() -> i64 {
    fn h<A, B: nogc>(a: A, b: B) -> i64 { return 0 }
    return 0
}
fn main() -> i64 { let v = vec[1, 2]; return h(v, 1) + outer() }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("`A` of `h`") && err.contains("vec[i64]"),
        "the bound on the FIRST type param must survive the shadow: {err}"
    );
}

#[test]
fn reject_shadow_with_different_type_param_names_cannot_weaken() {
    let err = reject(
        "\
nogc fn h<A, B>(a: A, b: B) -> i64 { let x = a; return 0 }
fn outer() -> i64 {
    nogc fn h<B>(b: B) -> i64 { return 0 }
    return 0
}
fn main() -> i64 { let v = vec[1, 2]; return h(v, 1) + outer() }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("`A` of `h`") && err.contains("vec[i64]"),
        "the wider signature's bound must survive the narrower shadow: {err}"
    );
}

#[test]
fn reject_sibling_nested_generics_sharing_a_bare_name() {
    let err = reject(
        "\
fn a() -> i64 {
    nogc fn h<A, B>(x: A, y: B) -> i64 { let z = x; return 0 }
    let v = vec[1, 2]
    return h(v, 1)
}
fn b() -> i64 {
    fn h<A, B: nogc>(x: A, y: B) -> i64 { return 0 }
    return 0
}
fn main() -> i64 { return a() + b() }
",
    );
    assert_code(&err, "E0730");
    assert!(
        err.contains("`A` of `h`") && err.contains("vec[i64]"),
        "a sibling scope's weaker signature must not erase the bound: {err}"
    );
}

#[test]
fn bare_name_collision_without_any_nogc_is_now_e0418() {
    let err = reject(
        "\
fn h<A, B>(a: A, b: B) -> i64 { let x = a; return 0 }
fn outer() -> i64 {
    fn h<A, B>(a: A, b: B) -> i64 { return 0 }
    return 0
}
fn main() -> i64 { let v = vec[1, 2]; return h(v, 1) + outer() }
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
fn helper<T>(x: T) -> i64 { return 0 }
fn outer() -> i64 {
    fn helper<U>(y: U) -> i64 { return 1 }
    let v = vec[1, 2]
    return helper(v)
}
fn main() -> i64 { return outer() }
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
nogc fn add(a: i64, b: i64) -> i64 { return a + b }
fn main() -> i64 { return add(1, 2) }
",
    );
}

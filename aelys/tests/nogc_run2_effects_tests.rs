use aelys_air::bir::build::build_program;
use aelys_air::bir::{Effect, EffectSet};
use aelys_driver::compile_to_typed_ast;

fn effects_of(src: &str, fn_name: &str) -> EffectSet {
    let program = compile_to_typed_ast(src).expect("source should type-check");
    let bir = build_program(&program);
    bir.bodies
        .iter()
        .find(|b| b.name == fn_name)
        .unwrap_or_else(|| panic!("no bir body named `{fn_name}`"))
        .intrinsic_effects
}

// `(*src).r` member node is rc-typed. missing this = a nogc fn silently reaching managed memory.
#[test]
fn critical1_managed_field_read_through_ref() {
    let src = "\
struct S { r: Rc<i64> }
fn f(src: &S) -> Rc<i64> {
    return (*src).r
}
";
    let eff = effects_of(src, "f");
    assert!(
        eff.contains(Effect::Managed),
        "reading a managed field through a ref carries Managed"
    );
    assert!(!eff.contains(Effect::Alloc), "a retain allocates nothing");
}

// the nogc keystone: a body over `&[i64]` doing bounds-checked arithmetic carries panic only,
#[test]
fn nogc_keystone_slice_arithmetic() {
    let src = r#"
fn sum(a: &[i64]) -> i64 { return a[0] + a[1] }
"#;
    let eff = effects_of(src, "sum");
    assert!(
        eff.contains(Effect::Panic),
        "indexing carries Panic (bounds check)"
    );
    assert!(
        !eff.contains(Effect::Managed),
        "a slice compute is nogc: Managed absent"
    );
    assert!(
        !eff.contains(Effect::Alloc),
        "no allocation in a slice compute"
    );
}

#[test]
fn managed_param_even_if_unused() {
    let src = r#"
fn f(r: Rc<i64>) -> i64 { return 5 }
"#;
    let eff = effects_of(src, "f");
    assert!(
        eff.contains(Effect::Managed),
        "an Rc param is released, so Managed"
    );
    assert!(
        !eff.contains(Effect::Alloc),
        "holding a param allocates nothing"
    );
}

#[test]
fn rc_new_is_managed_alloc() {
    let src = r#"
fn f() -> Rc<i64> { return Rc::new(5) }
"#;
    let eff = effects_of(src, "f");
    assert!(eff.contains(Effect::Managed));
    assert!(eff.contains(Effect::Alloc));
}

#[test]
fn vec_literal_is_managed_alloc() {
    let src = "\
fn f() -> i64 {
    let v = vec[1, 2, 3]
    return v[0]
}
";
    let eff = effects_of(src, "f");
    assert!(eff.contains(Effect::Managed));
    assert!(eff.contains(Effect::Alloc));
}

#[test]
fn vec_push_is_managed_alloc() {
    let src = "\
fn push_one(v: Vec<i64>) -> i64 {
    Vec::push(v, 9)
    return v[0]
}
";
    let eff = effects_of(src, "push_one");
    assert!(eff.contains(Effect::Managed), "a Vec param is managed");
    assert!(
        eff.contains(Effect::Alloc),
        "Vec::push grows on the managed heap"
    );
}

#[test]
fn discarded_managed_result_is_managed() {
    let src = "\
fn make_rc() -> Rc<i64> {
    return Rc::new(1)
}
fn dump() -> i64 {
    make_rc()
    return 0
}
";
    let eff = effects_of(src, "dump");
    assert!(
        eff.contains(Effect::Managed),
        "a discarded Rc temporary is released"
    );
    assert!(
        !eff.contains(Effect::Alloc),
        "the callee's alloc is a stage-2 summary, not intrinsic"
    );
}

#[test]
fn div_and_mod_are_panic() {
    let div = effects_of("fn d(a: i64, b: i64) -> i64 { return a / b }", "d");
    assert_eq!(
        div,
        EffectSet::EMPTY.with(Effect::Panic),
        "div carries only Panic"
    );

    let modu = effects_of("fn m(a: i64, b: i64) -> i64 { return a % b }", "m");
    assert_eq!(
        modu,
        EffectSet::EMPTY.with(Effect::Panic),
        "mod carries only Panic"
    );
}

// a slice range is bounds-checked, so it panics; primitive elements keep it nogc.
#[test]
fn slice_range_is_panic_not_managed() {
    let src = "\
fn sl(a: &[i64]) -> i64 {
    let s = a[0..2]
    return s[0]
}
";
    let eff = effects_of(src, "sl");
    assert!(eff.contains(Effect::Panic));
    assert!(!eff.contains(Effect::Managed));
}

#[test]
fn unwrap_panic_arm_is_panic() {
    let src = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }
fn ok_val() -> Result<i64, E> { return Result::Ok(9) }
fn unwrapper() -> i64 { return ok_val().unwrap() }
"#;
    let eff = effects_of(src, "unwrapper");
    assert!(eff.contains(Effect::Panic), "the Err arm panics");
    assert!(
        !eff.contains(Effect::Managed),
        "a Result of primitives is not managed"
    );
}

#[test]
fn pure_arithmetic_is_empty() {
    let src = r#"
fn pure(a: i64, b: i64) -> i64 { return a + b * a - b }
"#;
    assert_eq!(
        effects_of(src, "pure"),
        EffectSet::EMPTY,
        "pure arithmetic is effect-free"
    );
}

#[test]
fn capturing_closure_is_managed_alloc() {
    let src = r#"
fn call_it(f: fn() -> i64) -> i64 { return f() }
fn make() -> i64 {
    let x: i64 = 5
    let g = fn() -> i64 { return x }
    return call_it(g)
}
"#;
    let eff = effects_of(src, "make");
    assert!(
        eff.contains(Effect::Managed),
        "the captured env lives on the managed heap"
    );
    assert!(
        eff.contains(Effect::Alloc),
        "constructing the closure env allocates"
    );
    assert!(
        !eff.contains(Effect::Panic),
        "no bounds-checked or dividing op in the constructor"
    );
}

#[test]
fn effectset_algebra() {
    let managed = EffectSet::EMPTY.with(Effect::Managed);
    let panic = EffectSet::EMPTY.with(Effect::Panic);
    let both = managed.union(panic);
    assert!(both.contains(Effect::Managed) && both.contains(Effect::Panic));
    assert!(managed.is_subset_of(both));
    assert!(!both.is_subset_of(managed));
    assert!(EffectSet::EMPTY.is_nogc());
    assert!(!managed.is_nogc());
    assert!(panic.is_nogc(), "Panic alone is still nogc");
}

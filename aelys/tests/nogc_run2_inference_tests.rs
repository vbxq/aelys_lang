use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

use aelys_air::bir::build::build_program;
use aelys_air::bir::{Effect, EffectSet, effect_summaries};
use aelys_driver::compile_to_typed_ast;

// top: the reachable top an indirect/unknown call floors to (mirrors the pass-internal const).
fn top() -> EffectSet {
    EffectSet::EMPTY
        .with(Effect::Managed)
        .with(Effect::Alloc)
        .with(Effect::Panic)
}

fn summaries(src: &str) -> HashMap<String, EffectSet> {
    let program = compile_to_typed_ast(src).expect("source should type-check");
    let bir = build_program(&program);
    effect_summaries(&bir)
}

fn summary_of(src: &str, fn_name: &str) -> EffectSet {
    *summaries(src)
        .get(fn_name)
        .unwrap_or_else(|| panic!("no summary for `{fn_name}`"))
}

#[test]
fn dump_inherits_alloc_from_make_rc() {
    let src = "\
fn make_rc() -> Rc<i64> {
    return Rc::new(1)
}
fn dump() -> i64 {
    make_rc()
    return 0
}
";
    let eff = summary_of(src, "dump");
    assert!(
        eff.contains(Effect::Managed),
        "dump releases the discarded Rc temporary"
    );
    assert!(
        eff.contains(Effect::Alloc),
        "dump inherits Alloc from make_rc via propagation (intrinsic had none)"
    );
}

#[test]
fn pure_chain_is_empty() {
    let src = "\
fn c() -> i64 { return 3 }
fn b() -> i64 { return c() }
fn a() -> i64 { return b() }
";
    let s = summaries(src);
    assert_eq!(s["a"], EffectSet::EMPTY, "a is pure through the chain");
    assert_eq!(s["b"], EffectSet::EMPTY, "b is pure");
    assert_eq!(s["c"], EffectSet::EMPTY, "c is pure");
}

#[test]
fn managed_propagates_transitively() {
    let src = "\
fn leaf() -> Rc<i64> { return Rc::new(0) }
fn helper() -> Rc<i64> { return leaf() }
fn root() -> i64 {
    helper()
    return 0
}
";
    let eff = summary_of(src, "root");
    assert!(
        eff.contains(Effect::Managed),
        "root reaches managed memory two calls deep"
    );
    assert!(
        eff.contains(Effect::Alloc),
        "Alloc propagates from leaf through helper to root"
    );
}

#[test]
fn pure_self_recursion_is_empty() {
    let src = "\
fn fib(n: i64) -> i64 {
    if n < 2 {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}
";
    assert_eq!(
        summary_of(src, "fib"),
        EffectSet::EMPTY,
        "pure recursion stays effect-free"
    );
}

// direct recursion, managed: the fixpoint keeps managed across the self-edge, and does not invent
#[test]
fn managed_self_recursion_is_managed_not_alloc() {
    let src = "\
fn mrec(n: i64, r: Rc<i64>) -> i64 {
    if n < 1 {
        return 0
    }
    return mrec(n - 1, r)
}
";
    let eff = summary_of(src, "mrec");
    assert!(
        eff.contains(Effect::Managed),
        "the Rc param is released, self-recursion preserves it"
    );
    assert!(
        !eff.contains(Effect::Alloc),
        "no construction, so no Alloc despite recursion"
    );
}

// mutual recursion (an scc): ping <-> pong, only pong allocates; the fixpoint over the cycle gives
#[test]
fn mutual_recursion_shares_effects_both_ways() {
    let src = "\
fn ping(n: i64) -> i64 {
    if n < 1 {
        return 0
    }
    return pong(n - 1)
}
fn pong(n: i64) -> i64 {
    if n < 1 {
        let r = Rc::new(0)
        return 0
    }
    return ping(n - 1)
}
";
    let s = summaries(src);
    assert!(
        s["pong"].contains(Effect::Managed) && s["pong"].contains(Effect::Alloc),
        "pong allocates"
    );
    assert!(
        s["ping"].contains(Effect::Managed) && s["ping"].contains(Effect::Alloc),
        "ping inherits Managed+Alloc across the mutual-recursion cycle"
    );
}

#[test]
fn indirect_call_is_top() {
    let src = "\
fn caller() -> i64 {
    let f = fn() -> i64 { return 1 }
    return f()
}
";
    assert_eq!(
        summary_of(src, "caller"),
        top(),
        "an indirect call floors the caller to TOP"
    );
}

#[test]
fn allocating_closure_makes_caller_managed_via_top() {
    let src = "\
fn caller() -> i64 {
    let f = fn() -> i64 {
        let v = vec[1, 2, 3]
        return v[0]
    }
    return f()
}
";
    let eff = summary_of(src, "caller");
    assert!(
        eff.contains(Effect::Managed),
        "the closure's managed work is caught by the TOP floor"
    );
}

#[test]
fn elision_eligible_helper_still_managed() {
    let src = "\
fn produce() -> Rc<i64> { return Rc::new(7) }
fn consumer() -> i64 {
    produce()
    return 0
}
";
    let s = summaries(src);
    assert!(
        s["produce"].contains(Effect::Managed),
        "the helper's returned-then-dropped Rc is elision-eligible yet still Managed pre-elision"
    );
    assert!(
        s["consumer"].contains(Effect::Managed),
        "consumer inherits Managed from produce"
    );
    assert!(
        s["consumer"].contains(Effect::Alloc),
        "consumer's only Alloc source is the propagated summary of produce"
    );
}

#[test]
fn critical1_global_shadow_does_not_underreport() {
    let src = "\
fn foo() -> i64 { return 0 }
fn sneaky() -> i64 {
    let v = Rc::new(0)
    return 42
}
let foo: fn() -> i64 = sneaky
fn caller() -> i64 { return foo() }
";
    let eff = summary_of(src, "caller");
    assert!(
        eff.contains(Effect::Managed),
        "a global shadowing a fn is an indirect call: caller must not under-report to EMPTY"
    );
    assert!(
        eff.contains(Effect::Alloc),
        "the indirect floor carries Alloc too"
    );
}

#[test]
fn corpus_no_panic_smoke() {
    let corpus = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("tests_e2e");

    let mut processed = 0usize;
    for entry in fs::read_dir(&corpus).expect("tests_e2e dir readable") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("aelys") {
            continue;
        }
        let src = match fs::read_to_string(&path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        if let Ok(program) = compile_to_typed_ast(&src) {
            let bir = build_program(&program);
            let _ = effect_summaries(&bir);
            processed += 1;
        }
    }
    assert!(
        processed >= 50,
        "expected the fixture corpus to be reachable, only ran {processed}"
    );
}

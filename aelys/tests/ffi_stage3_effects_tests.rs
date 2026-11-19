use aelys_air::bir::{Effect, EffectSet, build, effect_summaries};
use aelys_driver::lower_file_to_air;
use aelys_opt::OptimizationLevel;
use std::fs;
use tempfile::tempdir;

const SIX: [Effect; 6] = [
    Effect::Managed,
    Effect::Alloc,
    Effect::Panic,
    Effect::Unwind,
    Effect::Block,
    Effect::Io,
];

const RESERVE: &str = "clause held under the reserve of the lambda body, see S3-C6";

fn rejects(id: &str, source: &str, code: &str) {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, source).expect("write fixture");
    let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
        Ok(_) => panic!("{id}: MUST be rejected ({RESERVE})\n{source}"),
        Err(rendered) => rendered,
    };
    assert!(
        rendered.contains(code),
        "{id}: the rejection MUST be {code} ({RESERVE})\n{source}\nrendered:\n{rendered}"
    );
}

fn summary_of(id: &str, source: &str, name: &str) -> EffectSet {
    let program = match aelys_driver::compile_to_typed_ast(source) {
        Ok(program) => program,
        Err(err) => panic!("{id}: MUST type-check\nerror:\n{err}"),
    };
    let bir = build::build_program(&program);
    *effect_summaries(&bir)
        .get(name)
        .unwrap_or_else(|| panic!("{id}: no summary for `{name}`"))
}

fn pin(id: &str, eff: EffectSet, expected: [bool; 6]) {
    for (effect, want) in SIX.iter().zip(expected) {
        assert_eq!(
            eff.contains(*effect),
            want,
            "{id}: {effect:?} should be {want}"
        );
    }
}

const A_BARE: &str = "nogc fn c() -> i64 {\n    let v = vec[1, 2, 3]\n    return 7\n}\nfn main() -> i64 { return c() }\n";
const A_UNSAFE: &str = "nogc fn c() -> i64 {\n    unsafe { let v = vec[1, 2, 3] }\n    return 7\n}\nfn main() -> i64 { return c() }\n";

#[test]
fn s3_c1_unsafe_does_not_re_enable_managed_allocation_in_nogc() {
    rejects("S3-C1", A_UNSAFE, "E0727");
}

#[test]
fn s3_c1_bis_the_bare_twin_is_rejected_the_same_way() {
    rejects("S3-C1'", A_BARE, "E0727");
}

const B_BARE: &str = "fn m() -> i64 {\n    let v = vec[1, 2, 3]\n    return 7\n}\nnogc fn c() -> i64 {\n    return m()\n}\nfn main() -> i64 { return c() }\n";
const B_UNSAFE: &str = "fn m() -> i64 {\n    let v = vec[1, 2, 3]\n    return 7\n}\nnogc fn c() -> i64 {\n    unsafe { return m() }\n}\nfn main() -> i64 { return c() }\n";

#[test]
fn s3_c2_unsafe_does_not_let_nogc_call_managed_code() {
    rejects("S3-C2", B_UNSAFE, "E0727");
}

#[test]
fn s3_c2_bis_the_bare_twin_is_rejected_the_same_way() {
    rejects("S3-C2'", B_BARE, "E0727");
}

const C_BARE: &str = "fn c() -> i64 {\n    let v = vec[1, 2, 3]\n    return v[0]\n}\nfn main() -> i64 { return c() }\n";
const C_UNSAFE: &str = "fn c() -> i64 {\n    unsafe {\n        let v = vec[1, 2, 3]\n        return v[0]\n    }\n}\nfn main() -> i64 { return c() }\n";

#[test]
fn s3_c3_the_effect_walk_descends_into_a_nested_block() {
    let bare = summary_of("S3-C3", C_BARE, "c");
    let under = summary_of("S3-C3", C_UNSAFE, "c");
    pin("S3-C3 bare", bare, [true, true, true, false, false, false]);
    pin("S3-C3 unsafe", under, [true, true, true, false, false, false]);
    for effect in SIX {
        assert_eq!(
            bare.contains(effect),
            under.contains(effect),
            "S3-C3: {effect:?} moved under `unsafe`, which would remove or add an effect ({RESERVE})"
        );
    }
}

const D_BARE: &str = "fn main() -> i64 {\n    let mut a: i64 = 7\n    let r: &mut i64 = &mut a\n    let s: &mut i64 = &mut a\n    return *r\n}\n";
const D_UNSAFE: &str = "fn main() -> i64 {\n    let mut a: i64 = 7\n    unsafe {\n        let r: &mut i64 = &mut a\n        let s: &mut i64 = &mut a\n        return *r\n    }\n}\n";

#[test]
fn s3_c4_unsafe_does_not_drop_a_live_guard() {
    rejects("S3-C4", D_UNSAFE, "E0713");
}

#[test]
fn s3_c4_bis_the_bare_twin_is_rejected_the_same_way() {
    rejects("S3-C4'", D_BARE, "E0713");
}

const E_UNSAFE: &str = "fn m() -> i64 {\n    let v = vec[1, 2, 3]\n    return 7\n}\nfn take(f: nogc fn() -> i64) -> i64 {\n    return f()\n}\nfn main() -> i64 {\n    unsafe { return take(m) }\n}\n";

#[test]
fn s3_c5_the_nogc_bound_is_the_second_route_of_the_managed_clause() {
    rejects("S3-C5", E_UNSAFE, "E0729");
}

#[test]
fn s3_c6_a_managed_allocation_in_a_lambda_body_of_a_nogc_fn_still_compiles() {
    let source = "nogc fn c() -> i64 {\n    let f = fn() -> i64 {\n        let v = vec[1, 2, 3]\n        return 7\n    }\n    return 7\n}\nfn main() -> i64 { return c() }\n";
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, source).expect("write fixture");
    assert!(
        lower_file_to_air(&root, OptimizationLevel::None).is_ok(),
        "S3-C6: the effect walk does not enter a lambda body, and this row records that hole \
         rather than claiming clauses (a), (b) and (c) closed; it is A14, it predates `unsafe` \
         and the ffi, and it is out of this run's scope\n{source}"
    );
}

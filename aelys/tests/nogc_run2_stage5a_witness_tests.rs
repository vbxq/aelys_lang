
use aelys_air::bir::build::build_program;
use aelys_air::bir::{effect_summaries, managed_chain, Step, StepKind};
use aelys_driver::{compile_to_typed_ast, lower_file_to_air};
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

fn assert_e0727(err: &str) {
    assert!(err.contains("[E0727]"), "must carry the E0727 code: {err}");
    assert!(err.contains("[nogc]"), "must carry the nogc marker: {err}");
    assert!(
        err.contains("declared nogc but its inferred effects reach managed memory"),
        "must be the nogc-reaches-managed diagnostic: {err}"
    );
}

fn has(err: &str, needle: &str) {
    assert!(err.contains(needle), "expected `{needle}` in:\n{err}");
}

fn lacks(err: &str, needle: &str) {
    assert!(!err.contains(needle), "did not expect `{needle}` in:\n{err}");
}

fn chain_of(src: &str, root: &str) -> Vec<Step> {
    let program = compile_to_typed_ast(src).expect("source should type-check");
    let bir = build_program(&program);
    let eff = effect_summaries(&bir);
    let body = bir
        .bodies
        .iter()
        .find(|b| b.name == root)
        .unwrap_or_else(|| panic!("no bir body named `{root}`"));
    managed_chain(&bir, &eff, body)
}

const THREE_HOP: &str = "\
fn leaf() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    return 0
}
fn helper() -> i64 {
    return leaf()
}
nogc fn f() -> i64 {
    return helper()
}
fn main() -> i64 { return f() }
";

#[test]
fn three_hop_chain_names_every_hop() {
    let err = reject(THREE_HOP);
    assert_e0727(&err);
    has(&err, "via `f -> helper -> leaf -> Vec::new`");
    has(&err, "module.aelys:2:17");
    has(&err, "^^^^^^^^^^ managed memory reached here");
    has(&err, "`f` is declared nogc here");
    has(&err, "calls `helper` here");
    has(&err, "calls `leaf` here");
    has(&err, "help: keep the value on the stack");
}

#[test]
fn twin_three_hop_compiles() {
    accepts(&THREE_HOP.replace("nogc fn f", "fn f"));
}

#[test]
fn direct_one_hop_anchors_on_the_operation() {
    let err = reject(
        "\
nogc fn f() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    return 0
}
fn main() -> i64 { return f() }
",
    );
    assert_e0727(&err);
    has(&err, "via `f -> Vec::new`");
    has(&err, "module.aelys:2:17");
    has(&err, "`f` is declared nogc here");
}

#[test]
fn naming_trap_names_the_operation_not_the_local_type() {
    let err = reject(
        "\
nogc fn f() -> i64 {
    let mut v = Vec::new()
    return 0
}
fn main() -> i64 { return f() }
",
    );
    assert_e0727(&err);
    has(&err, "via `f -> Vec::new`");
    lacks(&err, "the managed local");
    lacks(&err, "a managed value");
}

#[test]
fn managed_parameter_is_named_when_no_operation_exists() {
    let err = reject(
        "\
nogc fn f(v: Vec<i64>) -> i64 { return 0 }
fn main() -> i64 { return 0 }
",
    );
    assert_e0727(&err);
    has(&err, "via `f -> the managed parameter `v``");
    has(&err, "module.aelys:1:11");
}

#[test]
fn transitive_chain_descends_past_the_returned_value() {
    let err = reject(
        "\
fn g() -> Rc<i64> { return Rc::new(0) }
nogc fn f() -> i64 {
    g()
    return 0
}
fn main() -> i64 { return f() }
",
    );
    assert_e0727(&err);
    has(&err, "via `f -> g -> Rc::new`");
    has(&err, "module.aelys:1:28");
    has(&err, "calls `g` here");
}

const MUTUAL: &str = "\
fn a(n: i64) -> i64 { return b(n) }
fn b(n: i64) -> i64 {
    a(n)
    return c(n)
}
fn c(n: i64) -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    return 0
}
nogc fn f() -> i64 { return a(1) }
fn main() -> i64 { return f() }
";

#[test]
fn mutual_recursion_terminates_with_a_sane_chain() {
    let err = reject(MUTUAL);
    assert_e0727(&err);
    has(&err, "via `f -> a -> b -> c -> Vec::new`");
    has(&err, "module.aelys:7:17");
    let names: Vec<String> = chain_of(MUTUAL, "f").iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, vec!["f", "a", "b", "c", "Vec::new"], "no name may repeat");
}

#[test]
fn twin_mutual_recursion_compiles() {
    accepts(&MUTUAL.replace("nogc fn f", "fn f"));
}

const PAST_CULPRIT: &str = "\
fn a() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    return b()
}
fn b() -> i64 { return a() }
nogc fn f() -> i64 { return a() }
fn main() -> i64 { return f() }
";

#[test]
fn descent_backtracks_to_the_allocation_it_walked_past() {
    let err = reject(PAST_CULPRIT);
    assert_e0727(&err);
    has(&err, "via `f -> a -> Vec::new`");
    lacks(&err, "via `f -> a -> b`");
    lacks(&err, "calls `b` here");
    has(&err, "module.aelys:2:17");
    has(&err, "^^^^^^^^^^ managed memory reached here");

    let chain = chain_of(PAST_CULPRIT, "f");
    let names: Vec<String> = chain.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, vec!["f", "a", "Vec::new"], "the unverified tail hop must be trimmed");
    assert_eq!(chain[2].kind, StepKind::Operation, "the operation must be named");
}

#[test]
fn twin_descent_past_the_culprit_compiles() {
    accepts(&PAST_CULPRIT.replace("nogc fn f", "fn f"));
}

// two remedies that cannot be carried out on a target with no name are not offered.
#[test]
fn indirect_builtin_call_gets_the_honest_wording() {
    let err = reject(
        "\
nogc fn f() -> i64 {
    println(\"hi\")
    return 0
}
fn main() -> i64 { return f() }
",
    );
    assert_e0727(&err);
    has(&err, "via `f -> <indirect call>`");
    has(
        &err,
        "note: the call target here is not statically known, so its effects are conservatively \
         assumed to reach managed memory",
    );
    has(&err, "help: call a function by name so its effects can be checked, or drop `nogc` from `f`");
// the caret must not assert an allocation the compiler never observed
    lacks(&err, "managed memory reached here");
    has(&err, "effects assumed to reach managed memory here");
// `println` is a builtin outside `fn_names`, so it cannot be made nogc; there is no value either
    lacks(&err, "keep the value on the stack");
    lacks(&err, "make every function on this path nogc");
    has(&err, "module.aelys:2:5");
}

#[test]
fn indirect_closure_call_gets_the_honest_wording() {
    let err = reject(
        "\
nogc fn f() -> i64 {
    let c = fn() -> i64 {
        let v = vec[1, 2, 3]
        return v[0]
    }
    return c()
}
fn main() -> i64 { return f() }
",
    );
    assert_e0727(&err);
    has(&err, "via `f -> <indirect call>`");
    has(
        &err,
        "note: the call target here is not statically known, so its effects are conservatively \
         assumed to reach managed memory",
    );
    lacks(&err, "managed memory reached here");
    has(&err, "effects assumed to reach managed memory here");
// e0728 rejects `nogc fn` outside an immutable parameter type, so a local closure cannot be one
    lacks(&err, "make every function on this path nogc");
    has(&err, "module.aelys:6:12");
}

const COLLISION: &str = "\
fn outer() -> i64 {
    fn dup() -> i64 {
        let mut v = Vec::new()
        Vec::push(v, 1)
        return 0
    }
    return dup()
}
fn other() -> i64 {
    fn dup() -> i64 { return 0 }
    return dup()
}
nogc fn f() -> i64 { return outer() }
fn main() -> i64 { return f() + other() }
";

#[test]
fn mid_chain_collision_truncates_at_the_last_verified_hop() {
    let chain = chain_of(COLLISION, "f");
    let names: Vec<String> = chain.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, vec!["f", "outer", "dup"]);
    assert!(chain[1].span.is_some(), "an unambiguous hop keeps its call site");
    assert_eq!(chain[2].kind, StepKind::Ambiguous, "the stop reason is not a hop");
    assert!(
        chain[2].span.is_none(),
        "a name shared by two bodies must carry no span"
    );

    let err = reject(COLLISION);
    assert_e0727(&err);
    has(&err, "via `f -> outer` (further steps are ambiguous: several functions are named `dup`)");
    lacks(&err, "via `f -> outer -> dup`");
    lacks(&err, "calls `dup` here");
    has(&err, "module.aelys:13:29");
}

const DIRECT_COLLISION: &str = "\
fn dup() -> i64 { return 0 }
fn holder() -> i64 {
    fn dup() -> i64 {
        let mut v = Vec::new()
        Vec::push(v, 1)
        return 0
    }
    return 0
}
nogc fn f() -> i64 { return dup() }
fn main() -> i64 { return f() + holder() }
";

#[test]
fn direct_collision_never_names_the_ambiguous_callee_as_a_hop() {
    let err = reject(DIRECT_COLLISION);
    assert_e0727(&err);
    lacks(&err, "via `f -> dup`");
    has(&err, "via `f` (further steps are ambiguous: several functions are named `dup`)");
    lacks(&err, "calls `dup` here");

    let chain = chain_of(DIRECT_COLLISION, "f");
    let names: Vec<String> = chain.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, vec!["f", "dup"]);
    assert_eq!(chain[1].kind, StepKind::Ambiguous);
    assert!(chain[1].span.is_none());
}

#[test]
fn shadowed_nogc_body_says_the_effects_were_merged() {
    let err = reject(
        "\
fn holder() -> i64 {
    fn f() -> i64 {
        let mut v = Vec::new()
        Vec::push(v, 1)
        return 0
    }
    return f()
}
nogc fn f() -> i64 { return 0 }
fn main() -> i64 { return f() + holder() }
",
    );
    assert_e0727(&err);
    lacks(&err, " via `");
    has(
        &err,
        "note: several functions in this program are named `f`, and the nogc check merges their \
         effects",
    );
}

// a deep chain must not produce a 1600-character header nor 200 secondary carets
#[test]
fn a_deep_chain_is_elided_and_its_carets_are_capped() {
    let mut src = String::from("fn h0() -> i64 {\n    let mut v = Vec::new()\n    Vec::push(v, 1)\n    return 0\n}\n");
    for i in 1..=200 {
        src.push_str(&format!("fn h{}() -> i64 {{ return h{}() }}\n", i, i - 1));
    }
    src.push_str("nogc fn f() -> i64 { return h200() }\nfn main() -> i64 { return f() }\n");

    let err = reject(&src);
    assert_e0727(&err);
    has(&err, "via `f -> h200 -> h199 -> ... (197 more) ... -> h1 -> h0 -> Vec::new`");
    let header = err.lines().next().expect("a first line");
    assert!(header.len() < 200, "the header must stay readable, got {}: {header}", header.len());
    let carets = err.matches("calls `").count();
    assert!(carets <= 3, "at most 3 hop carets, got {carets}:\n{err}");
    assert!(err.lines().count() < 40, "the whole render must stay short:\n{err}");
    assert_eq!(chain_of(&src, "f").len(), 203);
}

#[test]
fn rendered_chain_is_identical_across_compiles() {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, THREE_HOP).expect("write source");
    let render = || {
        lower_file_to_air(&source_path, OptimizationLevel::None)
            .map(|_| ())
            .expect_err("the nogc check must reject this program")
            .to_string()
    };
    let first = render();
    assert!(first.contains("via `f -> helper -> leaf -> Vec::new`"));
    for _ in 0..24 {
        assert_eq!(first, render(), "the rendered witness must not vary run to run");
    }
}

#[test]
fn reconstructed_chain_is_identical_across_runs() {
    let first = chain_of(MUTUAL, "f");
    for _ in 0..24 {
        assert_eq!(first, chain_of(MUTUAL, "f"), "the chain must not vary run to run");
    }
}

#[test]
fn accepted_programs_are_untouched() {
    accepts(
        "\
nogc fn sum3(s: &[i64]) -> i64 { return s[0] + s[1] + s[2] }
nogc fn helper(x: i64) -> i64 { return x + 1 }
nogc fn f() -> i64 { return helper(41) }
fn main() -> i64 { return f() }
",
    );
}


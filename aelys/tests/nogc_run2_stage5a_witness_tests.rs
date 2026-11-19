use aelys_air::bir::build::build_program;
use aelys_air::bir::{effect_summaries, managed_chain, Effect, Step, StepKind};
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
    assert!(
        !err.contains(needle),
        "did not expect `{needle}` in:\n{err}"
    );
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
use std::process::ExitCode; fn leaf() -> i64 {
    let mut v = Vec::new();
    Vec::push(v, 1);
    0
}
fn helper() -> i64 {
    leaf()
}
nogc fn f() -> i64 {
    helper()
}
fn main() -> ExitCode { return ExitCode::from(f() as u8); }
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
use std::process::ExitCode; nogc fn f() -> i64 {
    let mut v = Vec::new();
    Vec::push(v, 1);
    0
}
fn main() -> ExitCode { return ExitCode::from(f() as u8); }
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
use std::process::ExitCode; nogc fn f() -> i64 {
    let mut v: Vec<i64> = Vec::new();
    0
}
fn main() -> ExitCode { return ExitCode::from(f() as u8); }
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
nogc fn f(v: Vec<i64>) -> i64 { 0 }
fn main() { }
",
    );
    assert_e0727(&err);
    has(&err, "via `f -> the managed parameter `v``");
    has(&err, "module.aelys:1:11");
}

const MUTUAL: &str = "\
use std::process::ExitCode; fn a(n: i64) -> i64 { b(n) }
fn b(n: i64) -> i64 {
    a(n);
    c(n)
}
fn c(n: i64) -> i64 {
    let mut v = Vec::new();
    Vec::push(v, 1);
    0
}
nogc fn f() -> i64 { a(1) }
fn main() -> ExitCode { return ExitCode::from(f() as u8); }
";

#[test]
fn mutual_recursion_terminates_with_a_sane_chain() {
    let err = reject(MUTUAL);
    assert_e0727(&err);
    has(&err, "via `f -> a -> b -> c -> Vec::new`");
    has(&err, "module.aelys:7:17");
    let names: Vec<String> = chain_of(MUTUAL, "f")
        .iter()
        .map(|s| s.name.clone())
        .collect();
    assert_eq!(
        names,
        vec!["f", "a", "b", "c", "Vec::new"],
        "no name may repeat"
    );
}

#[test]
fn twin_mutual_recursion_compiles() {
    accepts(&MUTUAL.replace("nogc fn f", "fn f"));
}

const PAST_CULPRIT: &str = "\
use std::process::ExitCode; fn a() -> i64 {
    let mut v = Vec::new();
    Vec::push(v, 1);
    b()
}
fn b() -> i64 { a() }
nogc fn f() -> i64 { a() }
fn main() -> ExitCode { return ExitCode::from(f() as u8); }
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
    assert_eq!(
        names,
        vec!["f", "a", "Vec::new"],
        "the unverified tail hop must be trimmed"
    );
    assert_eq!(
        chain[2].kind,
        StepKind::Operation,
        "the operation must be named"
    );
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
use std::process::ExitCode; nogc fn f() -> i64 {
    println!(\"hi\");
    0
}
fn main() -> ExitCode { return ExitCode::from(f() as u8); }
",
    );
    assert_e0727(&err);
    has(&err, "via `f -> println`");
    lacks(&err, "<indirect call>");
    has(
        &err,
        "note: `println` is a built-in, so its effects are assumed to reach managed memory",
    );
    has(
        &err,
        "help: move the `println` out of `f`, or drop `nogc` from `f`",
    );
    // the caret must not assert an allocation the compiler never observed
    lacks(&err, "managed memory reached here");
    has(&err, "effects assumed to reach managed memory here");
    // `println` is a builtin outside `fn_names`, so it cannot be made nogc; there is no value either
    lacks(&err, "keep the value on the stack");
    lacks(&err, "make every function on this path nogc");
    has(&err, "module.aelys:2:5");
}

fn bodies_named(src: &str, name: &str) -> usize {
    let program = compile_to_typed_ast(src).expect("source should type-check");
    build_program(&program)
        .bodies
        .iter()
        .filter(|body| body.name == name)
        .count()
}

const DIRECT_COLLISION: &str = "\
use std::process::ExitCode; fn dup() -> i64 { 0 }
fn holder() -> i64 {
    let g = move || -> i64 {
        fn dup() -> i64 {
            let mut v = Vec::new();
            Vec::push(v, 1);
            0
        }
        0
    };
    g()
}
nogc fn f() -> i64 { dup() }
fn main() -> ExitCode { return ExitCode::from((f() + holder()) as u8); }
";

#[test]
fn direct_collision_never_names_the_ambiguous_callee_as_a_hop() {
    assert_eq!(
        bodies_named(DIRECT_COLLISION, "dup"),
        2,
        "the fixture must produce two bir bodies named `dup`"
    );

    let err = reject(DIRECT_COLLISION);
    assert_e0727(&err);
    lacks(&err, "via `f -> dup`");
    has(
        &err,
        "via `f` (further steps are ambiguous: several functions are named `dup`)",
    );
    lacks(&err, "calls `dup` here");

    let chain = chain_of(DIRECT_COLLISION, "f");
    let names: Vec<String> = chain.iter().map(|s| s.name.clone()).collect();
    assert_eq!(names, vec!["f", "dup"]);
    assert_eq!(chain[1].kind, StepKind::Ambiguous);
    assert!(chain[1].span.is_none());
}

const SHADOWED_NOGC: &str = "\
use std::process::ExitCode; fn holder() -> i64 {
    let g = move || -> i64 {
        fn f() -> i64 {
            let mut v = Vec::new();
            Vec::push(v, 1);
            0
        }
        0
    };
    g()
}
nogc fn f() -> i64 { 0 }
fn main() -> ExitCode { return ExitCode::from((f() + holder()) as u8); }
";

#[test]
fn shadowed_nogc_body_says_the_effects_were_merged() {
    assert_eq!(
        bodies_named(SHADOWED_NOGC, "f"),
        2,
        "the fixture must produce two bir bodies named `f`"
    );

    let program = compile_to_typed_ast(SHADOWED_NOGC).expect("source should type-check");
    let bir = build_program(&program);
    let declared = bir
        .bodies
        .iter()
        .find(|body| body.name == "f" && body.declared_nogc)
        .expect("one of the two bodies is the nogc declaration");
    assert!(
        !declared.intrinsic_effects.contains(Effect::Managed),
        "the nogc body reaches nothing managed on its own"
    );
    assert!(
        effect_summaries(&bir)
            .get("f")
            .expect("a summary for `f`")
            .contains(Effect::Managed),
        "the summary must carry the shadowing body's managed effect, which is the merge"
    );

    let err = reject(SHADOWED_NOGC);
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
    let mut src = String::from(
        "fn h0() -> i64 {\n    let mut v = Vec::new();\n    Vec::push(v, 1);\n    0\n}\n",
    );
    for i in 1..=200 {
        src.push_str(&format!("fn h{}() -> i64 {{ h{}() }}\n", i, i - 1));
    }
    src.push_str("use std::process::ExitCode; nogc fn f() -> i64 { h200() }\nfn main() -> ExitCode { return ExitCode::from(f() as u8); }\n");

    let err = reject(&src);
    assert_e0727(&err);
    has(
        &err,
        "via `f -> h200 -> h199 -> ... (197 more) ... -> h1 -> h0 -> Vec::new`",
    );
    let header = err.lines().next().expect("a first line");
    assert!(
        header.len() < 200,
        "the header must stay readable, got {}: {header}",
        header.len()
    );
    let carets = err.matches("calls `").count();
    assert!(carets <= 3, "at most 3 hop carets, got {carets}:\n{err}");
    assert!(
        err.lines().count() < 40,
        "the whole render must stay short:\n{err}"
    );
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
        assert_eq!(
            first,
            render(),
            "the rendered witness must not vary run to run"
        );
    }
}

#[test]
fn reconstructed_chain_is_identical_across_runs() {
    let first = chain_of(MUTUAL, "f");
    for _ in 0..24 {
        assert_eq!(
            first,
            chain_of(MUTUAL, "f"),
            "the chain must not vary run to run"
        );
    }
}

#[test]
fn accepted_programs_are_untouched() {
    accepts(
        "\
use std::process::ExitCode; nogc fn sum3(s: &[i64]) -> i64 { s[0] + s[1] + s[2] }
nogc fn helper(x: i64) -> i64 { x + 1 }
nogc fn f() -> i64 { helper(41) }
fn main() -> ExitCode { return ExitCode::from(f() as u8); }
",
    );
}

const STAGE0_MANAGED_COLLISION: &str = r#"
use std::process::ExitCode; fn dup() -> i64 { 0 }
fn outer() -> i64 {
    fn dup() -> i64 {
        let mut v = Vec::new();
        Vec::push(v, 1);
        0
    }
    dup()
}
nogc fn f() -> i64 { outer() }
fn main() -> ExitCode { return ExitCode::from(f() as u8); }
"#;

const STAGE0_EFFECT_FREE_SHADOW: &str = r#"
use std::process::ExitCode; fn dup() -> i64 { 0 }
fn outer() -> i64 {
    fn dup() -> i64 { 0 }
    dup()
}
nogc fn f() -> i64 { outer() }
fn main() -> ExitCode { return ExitCode::from(f() as u8); }
"#;

const STAGE0_RESERVED_MANAGED: &str = r#"
use std::process::ExitCode; nogc fn __f() -> i64 {
    let mut v = Vec::new();
    Vec::push(v, 1);
    0
}
fn main() -> ExitCode { return ExitCode::from(__f() as u8); }
"#;

const STAGE0_MULTIPLE_SHADOWS: &str = r#"
use std::process::ExitCode; fn dup() -> i64 { 0 }
fn first() -> i64 {
    fn dup() -> i64 { 1 }
    dup()
}
fn second() -> i64 {
    fn dup() -> i64 { 2 }
    dup()
}
fn main() -> ExitCode { return ExitCode::from((first() + second()) as u8); }
"#;

fn assert_code_before(err: &str, first: &str, second: &str) {
    let first_pos = err
        .find(&format!("[{first}]"))
        .unwrap_or_else(|| panic!("expected [{first}] in:\n{err}"));
    let second_pos = err
        .find(&format!("[{second}]"))
        .unwrap_or_else(|| panic!("expected [{second}] in:\n{err}"));
    assert!(
        first_pos < second_pos,
        "expected [{first}] before [{second}]:\n{err}"
    );
}

#[test]
fn managed_shadow_reports_nogc_before_e0418() {
    let err = reject(STAGE0_MANAGED_COLLISION);
    assert_code_before(&err, "E0727", "E0418");
}

#[test]
fn effect_free_shadow_stays_e0418_and_typed_ast_rejects() {
    let err = reject(STAGE0_EFFECT_FREE_SHADOW);
    has(&err, "[E0418]");
    lacks(&err, "[E0727]");
    let typed_err = match compile_to_typed_ast(STAGE0_EFFECT_FREE_SHADOW) {
        Ok(_) => panic!("compile_to_typed_ast must reject the shadow"),
        Err(err) => err,
    };
    has(&typed_err.to_string(), "[E0418]");
}

#[test]
fn reserved_managed_nogc_reports_e0727_before_reserved_name() {
    let err = reject(STAGE0_RESERVED_MANAGED);
    assert_code_before(&err, "E0727", "E0428");
}

#[test]
fn non_shadow_sema_errors_remain_fatal() {
    let err = reject("use std::process::ExitCode; fn main() -> ExitCode { return ExitCode::from(missing() as u8); }\n");
    has(&err, "[E0201]");
    lacks(&err, "[E0418]");
}

#[test]
fn multiple_shadow_diagnostics_are_preserved() {
    let err = reject(STAGE0_MULTIPLE_SHADOWS);
    assert_eq!(
        err.matches("[E0418]").count(),
        2,
        "both nested shadow diagnostics must survive:\n{err}"
    );
}

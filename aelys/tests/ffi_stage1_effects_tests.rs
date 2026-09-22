use aelys_air::bir::effects::{EXTERN_DEFAULT, EXTERN_NOGC};
use aelys_air::bir::{Effect, EffectSet, build, effect_summaries};
use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const SIX: [Effect; 6] = [
    Effect::Managed,
    Effect::Alloc,
    Effect::Panic,
    Effect::Unwind,
    Effect::Block,
    Effect::Io,
];

// gates the call site on , so every call below carries its own block
const NOGC_EXTERN: &str = "unsafe extern nogc fn e(x: i64) -> i64\n\nnogc fn c() -> i64 {\n    \
                           unsafe { return e(1) }\n}\n\nfn main() -> i64 {\n    return c()\n}\n";
const PLAIN_EXTERN: &str = "unsafe extern fn e(x: i64) -> i64\n\nnogc fn c() -> i64 {\n    \
                            unsafe { return e(1) }\n}\n\nfn main() -> i64 {\n    return c()\n}\n";
const NOGC_LIBC: &str = "unsafe extern nogc fn labs(x: i64) -> i64\n\nnogc fn c() -> i64 {\n    \
                         unsafe { return labs(-37) }\n}\n\nfn main() -> i64 {\n    return c()\n}\n";

const NOGC_EXTERN_BARE: &str = "unsafe extern nogc fn e(x: i64) -> i64\n\nnogc fn c() -> i64 {\n    \
                                return e(1)\n}\n\nfn main() -> i64 {\n    return c()\n}\n";
const PLAIN_EXTERN_BARE: &str = "unsafe extern fn e(x: i64) -> i64\n\nnogc fn c() -> i64 {\n    \
                                 return e(1)\n}\n\nfn main() -> i64 {\n    return c()\n}\n";
const NOGC_LIBC_BARE: &str = "unsafe extern nogc fn labs(x: i64) -> i64\n\nnogc fn c() -> i64 {\n    \
                              return labs(-37)\n}\n\nfn main() -> i64 {\n    return c()\n}\n";

fn rejected_as_e0617(id: &str, source: &str) {
    let dir = tempdir().expect("tempdir");
    let root = write_fixture(dir.path(), source);
    let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
        Ok(_) => panic!("{id}: the bare call MUST be rejected\n{source}"),
        Err(rendered) => rendered,
    };
    assert!(
        rendered.contains("E0617"),
        "{id}: the rejection MUST be E0617\nrendered:\n{rendered}"
    );
}

#[test]
fn f3_4_bis_the_old_bare_spelling_of_the_nogc_extern_call_is_now_e0617() {
    rejected_as_e0617("F3-4-bis", NOGC_EXTERN_BARE);
}

#[test]
fn f3_5_bis_the_old_bare_spelling_of_the_plain_extern_call_is_now_e0617() {
    rejected_as_e0617("F3-5-bis", PLAIN_EXTERN_BARE);
}

#[test]
fn f3_6_bis_the_old_bare_spelling_of_the_libc_call_is_now_e0617() {
    rejected_as_e0617("F3-6-bis", NOGC_LIBC_BARE);
}

fn write_fixture(dir: &Path, source: &str) -> PathBuf {
    let root = dir.join("root.aelys");
    fs::write(&root, source).expect("write fixture");
    root
}

fn typed(id: &str, source: &str) -> aelys_sema::TypedProgram {
    match aelys_driver::compile_to_typed_ast(source) {
        Ok(program) => program,
        Err(err) => panic!("{id}: MUST type-check\nerror:\n{err}"),
    }
}

fn summary_of(id: &str, source: &str, name: &str) -> EffectSet {
    let program = typed(id, source);
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

#[test]
fn f3_4_a_nogc_caller_of_a_nogc_extern_is_accepted() {
    for (level, opt) in LEVELS {
        let dir = tempdir().expect("tempdir");
        let root = write_fixture(dir.path(), NOGC_EXTERN);
        if let Err(rendered) = lower_file_to_air(&root, *opt) {
            panic!(
                "F3-4 at {level}: a nogc caller of a nogc declaration MUST be accepted\n\
                 rendered:\n{rendered}"
            );
        }
    }
}

#[test]
fn f3_4_the_nogc_extern_call_answers_at_runtime() {
    let dir = tempdir().expect("tempdir");
    let root = write_fixture(dir.path(), NOGC_LIBC);
    compile_file_with_llvm_variant(&root, OptimizationLevel::None, false, RuntimeVariant::Rc)
        .expect("F3-4: the nogc libc call MUST compile and link");
    let exe = {
        let mut o = root.with_extension("");
        if cfg!(windows) {
            o.set_extension("exe");
        }
        o
    };
    let out = Command::new(&exe).output().expect("run executable");
    assert_eq!(
        out.status.code(),
        Some(37),
        "F3-4: the nogc libc call MUST answer 37\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn f3_5_a_nogc_caller_of_a_plain_extern_is_rejected() {
    for (level, opt) in LEVELS {
        let dir = tempdir().expect("tempdir");
        let root = write_fixture(dir.path(), PLAIN_EXTERN);
        let rendered = match lower_file_to_air(&root, *opt) {
            Ok(_) => panic!(
                "F3-5 at {level}: a nogc caller of a plain declaration MUST be rejected\n\
                 {PLAIN_EXTERN}"
            ),
            Err(rendered) => rendered,
        };
        assert!(
            rendered.contains("E0727"),
            "F3-5 at {level}: the rejection MUST be E0727\nrendered:\n{rendered}"
        );
        assert!(
            rendered.contains("via `c -> e`"),
            "F3-5 at {level}: the chain MUST name the declaration it came from\n\
             rendered:\n{rendered}"
        );
    }
}

#[test]
fn f3_6_the_declaration_decides_the_six_bits_on_the_caller() {
    let nogc = summary_of("F3-6", NOGC_EXTERN, "c");
    pin("F3-6 nogc", nogc, [false, true, true, true, true, true]);
    assert_eq!(
        nogc, EXTERN_NOGC,
        "F3-6: a nogc declaration hands its caller exactly the nogc default"
    );
    assert!(
        nogc.contains(Effect::Alloc),
        "F3-6: nogc is not noalloc, an audited binding may still call malloc"
    );

    let plain = summary_of("F3-6", PLAIN_EXTERN, "c");
    pin("F3-6 plain", plain, [true, true, true, true, true, true]);
    assert_eq!(
        plain, EXTERN_DEFAULT,
        "F3-6: a plain declaration hands its caller exactly the declared default"
    );
}

#[test]
fn no_body_is_built_for_a_declaration() {
    let program = typed("F3-4", NOGC_EXTERN);
    let bir = build::build_program(&program);
    assert!(
        !bir.bodies.iter().any(|b| b.name == "e"),
        "a declaration MUST NOT get a body, the registry carries it instead"
    );
    let decl = bir
        .externs
        .get("e")
        .expect("the declaration MUST reach the registry");
    assert!(
        decl.declared_nogc,
        "the `nogc` claim MUST reach the registry from the declaration"
    );
    assert_eq!(decl.foreign.symbol, "e");

    let plain = build::build_program(&typed("F3-5", PLAIN_EXTERN));
    assert!(
        !plain.externs["e"].declared_nogc,
        "a declaration without `nogc` MUST NOT claim it"
    );
}

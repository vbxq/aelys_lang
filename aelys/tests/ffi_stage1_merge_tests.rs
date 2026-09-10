use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use aelys_air::bir::{ForeignSig, foreign_conflict, foreign_merge_clashes, foreign_signatures};
use aelys_opt::OptimizationLevel;
use aelys_sema::{
    InferType, TypeTable, TypedFunction, TypedProgram, TypedStmt, TypedStmtKind, TypedParam,
};
use aelys_syntax::{ForeignConv, ForeignDecl, Source, Span as SyntaxSpan};
use tempfile::tempdir;

type Files = &'static [(&'static str, &'static str)];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn stage(files: Files) -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir");
    for (name, body) in files {
        fs::write(dir.path().join(name), body).expect("write unit");
    }
    dir
}

struct Run {
    rendered: String,
    exit: Option<i32>,
}

fn run_root(files: Files) -> Run {
    let dir = stage(files);
    let root = dir.path().join("main.aelys");
    match aelys_driver::compile_file_with_llvm(&root, OptimizationLevel::None, false) {
        Err(err) => Run {
            rendered: err.to_string(),
            exit: None,
        },
        Ok(_) => {
            let exe = root.with_extension("");
            let status = Command::new(&exe).status().expect("run the linked program");
            Run {
                rendered: String::new(),
                exit: status.code(),
            }
        }
    }
}

fn registry(source: &str) -> HashMap<String, ForeignSig> {
    let program = aelys_driver::compile_to_typed_ast(source)
        .unwrap_or_else(|err| panic!("the unit MUST type-check\n{err}"));
    foreign_signatures(&program).0
}

const A_LABS: &str = "unsafe extern fn labs(x: i64) -> i64\n\npub fn use_a(x: i64) -> i64 {\n    unsafe { return labs(x) }\n}\n";
const MAIN_TWO: &str = "needs a\nneeds b\n\nfn main() -> i64 {\n    return a.use_a(-20) + b.use_b(-17)\n}\n";

fn b_labs(decl: &str, call: &str) -> String {
    format!("{decl}\n\npub fn use_b(x: i64) -> i64 {{\n    unsafe {{ return {call} }}\n}}\n")
}

// gates the call site on , so the old bare spelling is kept as a rejected twin
#[test]
fn f3_08_bis_the_old_bare_spelling_of_a_unit_body_call_is_now_e0617() {
    let bare = "unsafe extern fn labs(x: i64) -> i64\n\npub fn use_a(x: i64) -> i64 {\n    return labs(x)\n}\n";
    let rendered = match aelys_driver::compile_to_typed_ast(bare) {
        Ok(_) => panic!("F3-8-bis: the bare call MUST be rejected\n{bare}"),
        Err(rendered) => rendered.to_string(),
    };
    assert!(
        rendered.contains("E0617"),
        "F3-8-bis: the rejection MUST be E0617\nrendered:\n{rendered}"
    );
}

#[test]
fn f3_08_two_units_declaring_one_symbol_alike_do_not_clash_in_the_registry() {
    let a = registry(A_LABS);
    let b = registry(&b_labs("unsafe extern fn labs(x: i64) -> i64", "labs(x)"));
    assert!(
        foreign_merge_clashes(&a, &b).is_empty(),
        "two agreeing declarations of one symbol merge into one"
    );
}

#[test]
fn f3_08_two_units_declaring_one_symbol_alike_link_once_and_run() {
    let dir = stage(&[
        ("a.aelys", A_LABS),
        (
            "b.aelys",
            "unsafe extern fn labs(x: i64) -> i64\n\npub fn use_b(x: i64) -> i64 {\n    unsafe { return labs(x) }\n}\n",
        ),
        ("main.aelys", MAIN_TWO),
    ]);
    let root = dir.path().join("main.aelys");
    aelys_driver::compile_file_with_llvm(&root, OptimizationLevel::None, true)
        .expect("F3-8: two agreeing declarations MUST link");
    let ir = fs::read_to_string(root.with_extension("ll")).expect("read emitted .ll");
    assert_eq!(
        ir.matches("declare i64 @labs(").count(),
        1,
        "F3-8: the merged program declares the symbol exactly once, found:\n{ir}"
    );
    let run = run_root(&[
        ("a.aelys", A_LABS),
        (
            "b.aelys",
            "unsafe extern fn labs(x: i64) -> i64\n\npub fn use_b(x: i64) -> i64 {\n    unsafe { return labs(x) }\n}\n",
        ),
        ("main.aelys", MAIN_TWO),
    ]);
    assert_eq!(
        run.exit,
        Some(37),
        "F3-8: the merged call still answers, found:\n{}",
        run.rendered
    );
}

#[test]
fn f3_09_two_units_disagreeing_on_the_signature_clash_in_the_registry() {
    let a = registry(A_LABS);
    let b = registry(&b_labs(
        "unsafe extern fn labs(x: i64, y: i64) -> i64",
        "labs(x, x)",
    ));
    let clashes = foreign_merge_clashes(&a, &b);
    assert_eq!(clashes.len(), 1, "F3-9: the divergence MUST be seen");
    assert_eq!(clashes[0].name, "labs");
    assert!(!clashes[0].same_unit);
}

#[test]
fn f3_09_two_units_disagreeing_on_the_signature_are_e0612() {
    let run = run_root(&[
        ("a.aelys", A_LABS),
        (
            "b.aelys",
            "unsafe extern fn labs(x: i64, y: i64) -> i64\n\npub fn use_b(x: i64) -> i64 {\n    unsafe { return labs(x, x) }\n}\n",
        ),
        ("main.aelys", MAIN_TWO),
    ]);
    assert!(
        run.rendered.contains("E0612"),
        "F3-9: two units disagreeing about one symbol MUST be E0612, found:\n{}\nexit {:?}",
        run.rendered,
        run.exit
    );
}

#[test]
fn f3_10_a_nogc_claim_on_one_side_only_clashes_in_the_registry() {
    let a = registry(A_LABS);
    let b = registry(&b_labs("unsafe extern nogc fn labs(x: i64) -> i64", "labs(x)"));
    let clashes = foreign_merge_clashes(&a, &b);
    assert_eq!(clashes.len(), 1, "F3-10: the `nogc` divergence MUST be seen");
    assert!(
        clashes[0].reason.contains("nogc"),
        "F3-10: the reason names the claim, found {:?}",
        clashes[0].reason
    );
}

#[test]
fn f3_10_a_nogc_claim_on_one_side_only_is_e0612() {
    let run = run_root(&[
        ("a.aelys", A_LABS),
        (
            "b.aelys",
            "unsafe extern nogc fn labs(x: i64) -> i64\n\npub fn use_b(x: i64) -> i64 {\n    unsafe { return labs(x) }\n}\n",
        ),
        ("main.aelys", MAIN_TWO),
    ]);
    assert!(
        run.rendered.contains("E0612") && run.rendered.contains("nogc"),
        "F3-10: the verdict the lowered ir cannot render MUST be E0612, found:\n{}\nexit {:?}",
        run.rendered,
        run.exit
    );
}

const DIAMOND: Files = &[
    (
        "base.aelys",
        "unsafe extern fn labs(x: i64) -> i64\n\npub fn base_use(x: i64) -> i64 {\n    unsafe { return labs(x) }\n}\n",
    ),
    (
        "l.aelys",
        "needs base\n\nunsafe extern fn labs(x: i64) -> i64\n\npub fn l_use(x: i64) -> i64 {\n    unsafe { return labs(x) + base.base_use(x) }\n}\n",
    ),
    (
        "r.aelys",
        "needs base\n\nunsafe extern fn labs(x: i64) -> i64\n\npub fn r_use(x: i64) -> i64 {\n    unsafe { return labs(x) }\n}\n",
    ),
    (
        "main.aelys",
        "needs l\nneeds r\n\nunsafe extern fn labs(x: i64) -> i64\n\nfn main() -> i64 {\n    unsafe { return l.l_use(-10) + r.r_use(-10) + labs(-7) }\n}\n",
    ),
];

#[test]
fn f3_11_a_diamond_of_agreeing_declarations_does_not_clash_in_the_registry() {
    let one = registry("unsafe extern fn labs(x: i64) -> i64\n\nfn main() -> i64 {\n    unsafe { return labs(-1) }\n}\n");
    assert!(
        foreign_merge_clashes(&one, &one).is_empty(),
        "F3-11: a declaration reached twice down two arms of a diamond is one declaration"
    );
}

#[test]
fn f3_11_a_diamond_of_agreeing_declarations_links_and_runs() {
    let run = run_root(DIAMOND);
    assert_eq!(
        run.exit,
        Some(37),
        "F3-11: the diamond MUST link and run, found:\n{}",
        run.rendered
    );
}

#[test]
fn f3_12_prime_an_extern_in_a_module_against_a_root_body_is_a_duplicate_group() {
    let dir = stage(&[
        ("m.aelys", "unsafe extern fn strlen(x: i64) -> i64\n"),
        (
            "main.aelys",
            "needs m\n\nfn strlen(x: i64) -> i64 {\n    return x + 1\n}\n\nfn main() -> i64 {\n    return strlen(1)\n}\n",
        ),
    ]);
    let err = aelys_driver::lower_file_to_air(&dir.path().join("main.aelys"), OptimizationLevel::None)
        .err()
        .expect("F3-12': the group MUST be refused");
    let rendered = err.to_string();
    assert!(
        rendered.contains("E0612"),
        "F3-12': the verdict MUST be E0612, found:\n{rendered}"
    );
    assert!(
        rendered.contains("strlen"),
        "F3-12': the verdict MUST name the symbol, found:\n{rendered}"
    );
}

#[test]
fn f3_12_prime_an_extern_in_a_module_against_a_root_body_is_e0612() {
    let run = run_root(&[
        ("m.aelys", "unsafe extern fn strlen(x: i64) -> i64\n"),
        (
            "main.aelys",
            "needs m\n\nfn strlen(x: i64) -> i64 {\n    return x + 1\n}\n\nfn main() -> i64 {\n    return strlen(1)\n}\n",
        ),
    ]);
    assert!(
        run.rendered.contains("E0612"),
        "F3-12': the first positive control of E0612 reachable from source, found:\n{}\nexit {:?}",
        run.rendered,
        run.exit
    );
}

#[test]
fn f3_12_second_the_twin_under_another_symbol_links_and_runs() {
    let run = run_root(&[
        ("m.aelys", "unsafe extern fn strnlen_probe(x: i64) -> i64\n"),
        (
            "main.aelys",
            "needs m\n\nfn strlen(x: i64) -> i64 {\n    return x + 1\n}\n\nfn main() -> i64 {\n    return strlen(1)\n}\n",
        ),
    ]);
    assert_eq!(
        run.exit,
        Some(2),
        "F3-12\": renaming the declaration makes the same shape link and run, found:\n{}",
        run.rendered
    );
}

#[test]
fn f3_13_a_module_body_under_a_qualified_name_never_meets_the_registry() {
    let root = registry(
        "unsafe extern fn labs(x: i64) -> i64\n\nfn main() -> i64 {\n    unsafe { return labs(-1) }\n}\n",
    );
    let module = registry("pub fn labs(x: i64) -> i64 {\n    return x + 1\n}\n");
    assert!(
        module.is_empty(),
        "F3-13: an aelys body carries no foreign signature"
    );
    assert!(
        foreign_merge_clashes(&root, &module).is_empty(),
        "F3-13: a qualified module body is a different symbol"
    );
}

#[test]
fn f3_13_a_module_body_called_qualified_runs_beside_the_declaration() {
    let run = run_root(&[
        ("m.aelys", "pub fn labs(x: i64) -> i64 {\n    return x + 1\n}\n"),
        (
            "main.aelys",
            "needs m\n\nunsafe extern fn labs(x: i64) -> i64\n\nfn main() -> i64 {\n    unsafe { return m.labs(1) + labs(-35) }\n}\n",
        ),
    ]);
    assert_eq!(
        run.exit,
        Some(37),
        "F3-13: `m.labs` and the foreign `labs` are two symbols, found:\n{}",
        run.rendered
    );
}

#[test]
fn f3_14_a_module_file_declares_and_calls_its_own_extern() {
    let run = run_root(&[
        (
            "m.aelys",
            "unsafe extern fn labs(x: i64) -> i64\n\npub fn m_abs(x: i64) -> i64 {\n    unsafe { return labs(x) }\n}\n",
        ),
        (
            "main.aelys",
            "needs m\n\nfn main() -> i64 {\n    return m.m_abs(-7)\n}\n",
        ),
    ]);
    assert_eq!(
        run.exit,
        Some(7),
        "F3-14: the merged registry links the extern declared in the module file, found:\n{}",
        run.rendered
    );
}

#[test]
fn f3_14_bis_the_bare_call_in_the_module_file_is_e0617_at_the_module_file() {
    let run = run_root(&[
        (
            "m.aelys",
            "unsafe extern fn labs(x: i64) -> i64\n\npub fn m_abs(x: i64) -> i64 {\n    return labs(x)\n}\n",
        ),
        (
            "main.aelys",
            "needs m\n\nfn main() -> i64 {\n    return m.m_abs(-7)\n}\n",
        ),
    ]);
    assert_eq!(run.exit, None, "F3-14': the bare call MUST be rejected");
    assert!(
        run.rendered.contains("E0617"),
        "F3-14': the rejection MUST be E0617, found:\n{}",
        run.rendered
    );
    assert!(
        run.rendered.contains("m.aelys"),
        "F3-14': the caret MUST name the module file that wrote the call, found:\n{}",
        run.rendered
    );
}

fn foreign_declaration(name: &str, params: &[InferType], nogc: bool) -> TypedStmt {
    let span = SyntaxSpan::new(0, 1, 1, 1);
    TypedStmt {
        kind: TypedStmtKind::Function(TypedFunction {
            name: name.to_string(),
            type_params: Vec::new(),
            params: params
                .iter()
                .enumerate()
                .map(|(i, ty)| TypedParam {
                    name: format!("p{i}"),
                    mutable: false,
                    ty: ty.clone(),
                    span,
                })
                .collect(),
            return_type: InferType::I64,
            body: Vec::new(),
            decorators: Vec::new(),
            is_pub: false,
            declared_nogc: nogc,
            foreign: Some(ForeignDecl {
                symbol: name.to_string(),
                calling_conv: ForeignConv::C,
                is_unsafe: true,
                span,
            }),
            span,
            captures: Vec::new(),
        }),
        span,
    }
}

fn program_of(stmts: Vec<TypedStmt>) -> TypedProgram {
    TypedProgram {
        stmts,
        source: Source::new("<unit>", ""),
        type_table: TypeTable::new(),
    }
}

#[test]
fn f3_07_two_declarations_of_one_name_in_one_unit_clash_library_only() {
    let program = program_of(vec![
        foreign_declaration("probe", &[InferType::I64], false),
        foreign_declaration("probe", &[InferType::I64, InferType::I64], false),
    ]);
    let (sigs, clashes) = foreign_signatures(&program);
    assert_eq!(sigs.len(), 1, "F3-7: one name holds one signature");
    assert_eq!(clashes.len(), 1, "F3-7: the second declaration is a clash");
    assert!(
        clashes[0].same_unit,
        "F3-7: the clash is inside one unit, which is the half no source can reach"
    );
}

#[test]
fn f3_07_the_source_form_is_stopped_by_e0301_before_the_registry_sees_it() {
    let err = aelys_driver::compile_to_typed_ast(
        "unsafe extern fn probe(x: i64) -> i64\nunsafe extern fn probe(x: i64) -> i64\n\nfn main() -> i64 {\n    return 0\n}\n",
    )
    .err()
    .expect("F3-7: two homonymous declarations in one unit are refused");
    let rendered = err.to_string();
    assert!(
        rendered.contains("E0301"),
        "F3-7: inference gets there first, so the registry half stays library-only, found:\n{rendered}"
    );
}

#[test]
fn the_conflict_predicate_reads_the_typed_key_and_nothing_else() {
    let span = SyntaxSpan::new(0, 1, 1, 1);
    let base = ForeignSig {
        symbol: "probe".to_string(),
        calling_conv: ForeignConv::C,
        params: vec![InferType::I64],
        ret: InferType::I64,
        declared_nogc: false,
        span,
    };
    assert_eq!(foreign_conflict(&base, &base.clone()), None);

    let mut other = base.clone();
    other.span = SyntaxSpan::new(0, 9, 9, 9);
    assert_eq!(
        foreign_conflict(&base, &other),
        None,
        "two sites are not a disagreement"
    );

    let mut nogc = base.clone();
    nogc.declared_nogc = true;
    assert!(foreign_conflict(&base, &nogc).is_some());

    let mut renamed = base.clone();
    renamed.symbol = "other".to_string();
    assert!(foreign_conflict(&base, &renamed).is_some());

    let mut rc = base.clone();
    rc.params = vec![InferType::Rc(Box::new(InferType::I64))];
    let mut borrow = base.clone();
    borrow.params = vec![InferType::Ref {
        referent: Box::new(InferType::I64),
        mutable: false,
    }];
    assert!(foreign_conflict(&rc, &borrow).is_some());
}

// every air span is stamped `file: 0` at lowering, so a post-merge caret cannot point into a module file
#[test]
fn a_post_merge_caret_cannot_name_the_module_file_that_declared_the_symbol() {
    let run = run_root(&[
        ("m.aelys", "unsafe extern fn strlen(x: i64) -> i64\n"),
        (
            "main.aelys",
            "needs m\n\nfn strlen(x: i64) -> i64 {\n    return x + 1\n}\n\nfn main() -> i64 {\n    return strlen(1)\n}\n",
        ),
    ]);
    assert!(
        run.rendered.contains("E0612"),
        "the verdict is what this row asserts, found:\n{}",
        run.rendered
    );
    assert!(
        !run.rendered.contains("m.aelys"),
        "owed: the second site lives in `m.aelys` and no caret can reach it today, found:\n{}",
        run.rendered
    );
    let lower = fs::read_to_string(repo_root().join("air/src/lower/mod.rs")).expect("read lowering");
    assert!(
        lower.contains("file: 0,"),
        "the reason is that an air span carries no file"
    );
}

#[test]
fn the_air_level_entry_point_is_still_ungarded_by_the_registry() {
    let text = fs::read_to_string(repo_root().join("driver/src/api/llvm/mod.rs"))
        .expect("read the driver");
    let body = text
        .split_once("pub fn compile_air_program_to_executable(")
        .expect("the air level entry point exists")
        .1;
    let body = body.split_once("\npub fn ").map(|(b, _)| b).unwrap_or(body);
    for guard in [
        "foreign_signatures",
        "foreign_merge_clashes",
        "duplicate_symbols",
        "reserved_user_names",
    ] {
        assert!(
            !body.contains(guard),
            "the air level entry point takes a program the caller built, and `{guard}` never runs \
             on it: the guard is owed, not held"
        );
    }
}

#[test]
fn the_driver_asks_for_duplicate_symbols_at_three_sites() {
    let text = fs::read_to_string(repo_root().join("driver/src/api/llvm/mod.rs"))
        .expect("read the driver");
    let sites = text.matches("symbols::duplicate_symbols(").count();
    assert_eq!(
        sites, 3,
        "per unit, after the merge, and after mono; the second one is what makes F3-12' fire"
    );
}

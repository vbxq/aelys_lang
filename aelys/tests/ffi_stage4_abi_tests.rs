// the foreign type surface, the abi barrier's return half, and the splice guard on runtime symbols

use aelys_air::{
    AirBlock, AirEnumDef, AirEnumVariant, AirFunction, AirGlobal, AirLocal, AirProgram, AirStmt,
    AirStmtKind, AirTerminator, AirType, BlockId, Callee, CallingConv, FunctionAttribs, FunctionId,
    GcMode, InlineHint, LocalId, Operand, Place, Rvalue,
};
use aelys_air::symbols::BOOTSTRAP_BUILTIN_SYMBOLS;
use aelys_codegen::CodegenContext;
use aelys_driver::{
    RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air, resolve_aelys_core_lib,
};
use aelys_opt::OptimizationLevel;
use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;
use std::process::Command;
use tempfile::tempdir;

const LEVELS: [(&str, OptimizationLevel); 4] = [
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const MAIN: &str = "\nfn main() -> i64 { return 0 }\n";

fn lower(source: &str) -> Result<(), String> {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("probe.aelys");
    fs::write(&path, source).expect("write");
    lower_file_to_air(&path, OptimizationLevel::None)
        .map(|_| ())
        .map_err(|err| err.to_string())
}

fn rejected_e0615(id: &str, decl: &str) {
    let source = format!("{decl}{MAIN}");
    match lower(&source) {
        Ok(()) => panic!("{id}: `{decl}` must be rejected by E0615"),
        Err(rendered) => assert!(
            rendered.contains("E0615"),
            "{id}: the verdict must be E0615, found:\n{rendered}"
        ),
    }
}

fn accepted(id: &str, decl: &str) {
    let source = format!("{decl}{MAIN}");
    if let Err(rendered) = lower(&source) {
        panic!("{id}: `{decl}` must stay accepted, rejected with:\n{rendered}");
    }
}

#[test]
fn a_reference_in_return_position_is_refused_whatever_it_points_at() {
    rejected_e0615("T1", "unsafe extern fn probe() -> &i64");
    rejected_e0615("T2", "unsafe extern fn probe() -> &u8");
    rejected_e0615(
        "T8",
        "struct Opaque {\n    a: i64,\n}\n\nunsafe extern fn probe() -> &Opaque",
    );
    rejected_e0615("T14", "unsafe extern fn probe() -> & &i64");
}

#[test]
fn the_parameter_position_and_the_void_return_do_not_move() {
    accepted("T3", "unsafe extern fn probe(p: &i64)");
    accepted("T11", "unsafe extern fn probe(p: i64)");
    // a declaration with no argument spelling, so it pins the surface and never an actual call
    accepted("T13", "unsafe extern fn probe(p: & &i64) -> i64");
}

#[test]
fn the_pre_existing_rejections_are_unchanged() {
    rejected_e0615(
        "T6",
        "struct S {\n    a: i64,\n}\n\nunsafe extern fn probe() -> S",
    );
    rejected_e0615(
        "T7",
        "struct S {\n    a: i64,\n}\n\nunsafe extern fn probe(s: S) -> i64",
    );
    rejected_e0615("T9", "unsafe extern fn probe(p: &[i64])");
    rejected_e0615("T10", "unsafe extern fn probe(s: string)");
    rejected_e0615("T10", "unsafe extern fn probe(s: &string)");
    rejected_e0615("T12", "unsafe extern fn probe(p: void)");
}

// t15, t16: the two spellings that never existed
#[test]
fn the_glued_ampersand_and_the_type_alias_are_still_syntax_errors() {
    for (id, source) in [
        ("T15", "unsafe extern fn probe() -> &&i64"),
        ("T16", "type H = &i64"),
    ] {
        let rendered = lower(&format!("{source}{MAIN}")).expect_err(id);
        assert!(rendered.contains("E0101"), "{id}: {rendered}");
    }
}

// t18: an external declaration takes no type parameter, so no monomorphisation can carry a `&t` out
#[test]
fn a_generic_external_declaration_is_still_malformed() {
    let rendered = lower(&format!("unsafe extern fn probe<T>() -> &T{MAIN}")).expect_err("T18");
    assert!(rendered.contains("E0614"), "T18: {rendered}");
}

#[test]
fn the_malloc_free_dereference_program_is_now_rejected() {
    let source = "unsafe extern fn malloc(n: i64) -> &i64\n\
                  unsafe extern fn free(p: &i64)\n\
                  fn main() -> i64 {\n\
                  \u{20}   let p = unsafe { malloc(8) }\n\
                  \u{20}   unsafe { free(p) }\n\
                  \u{20}   return *p\n\
                  }\n";
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("fa.aelys");
    fs::write(&path, source).expect("write");
    for (name, opt) in LEVELS {
        let err = compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::Rc)
            .expect_err("F-A must not compile any more");
        assert!(
            err.to_string().contains("E0615"),
            "F-A {name}: the return type is the verdict: {err}"
        );
    }
    assert!(!dir.path().join("fa").exists(), "F-A: no executable");
}

#[test]
#[ignore = "the origin model of the reference design (charter section 4) is outside this run; \
            measured here, a `&buf[0]` on a local kept by the callee answers 123 at -O0 and 124 at -O1..3"]
fn a_borrowed_local_kept_by_the_callee_answers_differently_at_each_level() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("lib")).expect("lib dir");
    fs::write(
        root.join("stash.c"),
        "static const unsigned char *kept;\n\
         void aelys_stash(const unsigned char *p) { kept = p; }\n\
         long aelys_peek(void) { return kept[0] + kept[1]; }\n",
    )
    .expect("write c");
    for (program, args) in [
        ("clang", vec!["-c", "-o", "stash.o", "stash.c"]),
        ("ar", vec!["rcs", "lib/libstash.a", "stash.o"]),
    ] {
        let ok = Command::new(program)
            .args(&args)
            .current_dir(root)
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        assert!(ok, "`{program}` is required by this row");
    }
    let link = aelys_driver::LinkRequirement {
        search_paths: vec![root.join("lib")],
        libraries: vec!["stash".to_string()],
    };
    let source = "unsafe extern fn aelys_stash(p: &u8)\n\
                  unsafe extern fn aelys_peek() -> i64\n\
                  fn stash() { let mut buf: [u8; 4] = [100, 24, 0, 0]\n\
                  \u{20}            unsafe { aelys_stash(&buf[0]) } }\n\
                  fn main() -> i64 { stash()\n\
                  \u{20}                 unsafe { return aelys_peek() } }\n";
    let path = root.join("t19.aelys");
    fs::write(&path, source).expect("write");
    let mut answers = Vec::new();
    for (name, opt) in LEVELS {
        aelys_driver::compile_file_with_llvm_linked(&path, opt, false, RuntimeVariant::Rc, &link)
            .expect("compiles");
        let code = Command::new(root.join("t19"))
            .status()
            .expect("run")
            .code()
            .unwrap_or(-1);
        answers.push((name, code));
    }
    assert_eq!(
        answers[0].1, answers[1].1,
        "a type contract may never depend on the optimization level, and here it does: {answers:?}"
    );
}

fn default_attribs() -> FunctionAttribs {
    FunctionAttribs {
        inline: InlineHint::Default,
        no_gc: false,
        no_unwind: false,
        cold: false,
    }
}

fn declaration(name: &str, is_extern: bool, conv: CallingConv, ret_ty: AirType) -> AirFunction {
    AirFunction {
        id: FunctionId(0),
        name: name.to_string(),
        gc_mode: GcMode::Managed,
        type_params: vec![],
        params: vec![],
        ret_ty,
        locals: vec![],
        blocks: if is_extern {
            vec![]
        } else {
            vec![AirBlock {
                id: BlockId(0),
                stmts: vec![],
                terminator: AirTerminator::Unreachable,
            }]
        },
        is_extern,
        calling_conv: conv,
        attributes: default_attribs(),
        span: None,
    }
}

fn opt_enum() -> AirEnumDef {
    AirEnumDef {
        name: "Opt".to_string(),
        type_params: vec![],
        variants: vec![
            AirEnumVariant {
                name: "Some".to_string(),
                payload: vec![AirType::I64],
                tag: 0,
            },
            AirEnumVariant {
                name: "None".to_string(),
                payload: vec![],
                tag: 1,
            },
        ],
        span: None,
    }
}

fn program_of(functions: Vec<AirFunction>) -> AirProgram {
    AirProgram {
        functions,
        structs: vec![],
        enums: vec![opt_enum()],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: HashMap::from([(
            "Opt".to_string(),
            aelys_air::layout::TypeLayout { size: 16, align: 8 },
        )]),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    }
}

fn codegen(program: &AirProgram) -> Result<(), String> {
    let mut context = CodegenContext::new("stage4_abi");
    context.compile(program).map_err(|err| format!("{err:?}"))
}

#[test]
fn an_aggregate_return_on_the_c_convention_is_refused_on_this_target() {
    if cfg!(target_os = "windows") {
        return;
    }
    for (id, name, is_extern, ret_ty) in [
        ("C1", "c1", true, AirType::Str),
        ("C2", "c2", true, AirType::Enum("Opt".to_string())),
        ("C3", "c3", true, AirType::Vec(Box::new(AirType::I64))),
        ("C9", "c9", false, AirType::Str),
    ] {
        let program = program_of(vec![declaration(name, is_extern, CallingConv::C, ret_ty)]);
        let err = codegen(&program).expect_err(id);
        assert!(
            err.contains(name) && err.contains("no sret path"),
            "{id}: {err}"
        );
    }
}

#[test]
fn a_scalar_return_and_the_internal_convention_are_untouched() {
    for (id, name, conv, ret_ty) in [
        ("C4", "c4", CallingConv::C, AirType::I64),
        ("C5", "c5", CallingConv::C, AirType::Void),
        ("C6", "c6", CallingConv::Aelys, AirType::Str),
    ] {
        let program = program_of(vec![declaration(name, true, conv, ret_ty)]);
        codegen(&program).unwrap_or_else(|err| panic!("{id}: must stay accepted: {err}"));
    }
}

#[test]
fn the_parameter_half_of_the_barrier_still_answers() {
    for (id, name, ty) in [
        ("C7", "c7", AirType::Str),
        ("C8", "c8", AirType::Enum("Opt".to_string())),
    ] {
        let mut function = declaration(name, true, CallingConv::C, AirType::I64);
        function.params = vec![aelys_air::AirParam {
            id: LocalId(0),
            ty,
            name: "p".to_string(),
            span: None,
        }];
        let err = codegen(&program_of(vec![function])).expect_err(id);
        assert!(err.contains(name), "{id}: {err}");
    }
}

// c10: the toggle comes from the second airfunction, and airtype::fnptr stays outside is_abi_unsafe_type
#[test]
fn structural_row_the_indirect_fnptr_program_toggles_by_its_other_declaration() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("root");
    let functions =
        fs::read_to_string(root.join("codegen/src/lowering/functions.rs")).expect("read");
    let barrier = functions
        .split_once("pub(crate) fn is_abi_unsafe_type(")
        .expect("is_abi_unsafe_type")
        .1
        .split_once("\n}\n")
        .expect("body")
        .0;
    assert!(
        !barrier.contains("FnPtr"),
        "C10: a fnptr type is still not what the barrier looks at"
    );
    if cfg!(target_os = "windows") {
        return;
    }
    let program = program_of(vec![declaration(
        "get_opt",
        true,
        CallingConv::C,
        AirType::Enum("Opt".to_string()),
    )]);
    let err = codegen(&program).expect_err("C10");
    assert!(err.contains("get_opt"), "C10: {err}");
}

fn caller_of(callee: Callee, ret_ty: AirType, globals: Vec<AirGlobal>) -> AirProgram {
    let mut program = program_of(vec![AirFunction {
        id: FunctionId(0),
        name: "caller".to_string(),
        gc_mode: GcMode::Managed,
        type_params: vec![],
        params: vec![],
        ret_ty: AirType::I64,
        locals: vec![AirLocal {
            id: LocalId(0),
            ty: ret_ty,
            name: None,
            is_mut: false,
            span: None,
        }],
        blocks: vec![AirBlock {
            id: BlockId(0),
            stmts: vec![AirStmt {
                kind: AirStmtKind::Assign {
                    place: Place::Local(LocalId(0)),
                    rvalue: Rvalue::Call {
                        func: callee,
                        args: vec![],
                    },
                },
                span: None,
            }],
            terminator: AirTerminator::Return(Some(Operand::Const(
                aelys_air::AirConst::IntLiteral(0),
            ))),
        }],
        is_extern: false,
        calling_conv: CallingConv::Aelys,
        attributes: default_attribs(),
        span: None,
    }]);
    program.globals = globals;
    program
}

#[test]
fn a_vec_program_still_compiles_and_runs_at_every_level() {
    let source = "fn main() -> i64 {\n\
                  \u{20}   let mut v = Vec::new()\n\
                  \u{20}   Vec::push(v, 7)\n\
                  \u{20}   Vec::push(v, 9)\n\
                  \u{20}   let mut total = 0\n\
                  \u{20}   for i in 0..2 { total = total + 1 }\n\
                  \u{20}   return Vec::len(v) + total\n\
                  }\n";
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("e1.aelys");
    fs::write(&path, source).expect("write");
    for (name, opt) in LEVELS {
        compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::Rc)
            .unwrap_or_else(|err| panic!("E1 {name}: {err}"));
        let code = Command::new(dir.path().join("e1"))
            .status()
            .expect("run")
            .code()
            .unwrap_or(-1);
        assert_eq!(code, 4, "E1 {name}: two pushes and two iterations");
    }
}

#[test]
fn an_unknown_runtime_symbol_is_refused_instead_of_guessed() {
    let program = caller_of(
        Callee::Named("__aelys_inconnu".to_string()),
        AirType::I64,
        vec![],
    );
    let err = codegen(&program).expect_err("E2");
    assert!(
        err.contains("__aelys_inconnu") && err.contains("AD_HOC_RUNTIME_SYMBOLS"),
        "E2: {err}"
    );
}

#[test]
fn structural_row_the_global_prefixes_are_tested_in_generate_call_not_in_resolve_callee() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("root");
    let calls = fs::read_to_string(root.join("codegen/src/lowering/calls.rs")).expect("read");
    let generate = calls
        .split_once("fn generate_call(")
        .expect("generate_call")
        .1;
    let (generate_body, rest) = generate
        .split_once("fn resolve_callee(")
        .expect("resolve_callee follows");
    assert!(
        generate_body.contains("strip_prefix(GLOBAL_GET_PREFIX)")
            && generate_body.contains("strip_prefix(GLOBAL_SET_PREFIX)"),
        "E3: the prefix tests belong to generate_call"
    );
    assert!(
        rest.contains("AD_HOC_RUNTIME_SYMBOLS"),
        "E3: the guard belongs to resolve_callee"
    );
    assert!(
        !generate_body.contains("AD_HOC_RUNTIME_SYMBOLS"),
        "E3: the guard must not have moved in front of the prefix tests"
    );
}

#[test]
fn a_name_without_the_runtime_prefix_still_takes_the_ad_hoc_path() {
    let program = caller_of(
        Callee::Named("mafonction".to_string()),
        AirType::I64,
        vec![],
    );
    codegen(&program).expect("E4: the guard bites only `__aelys_`");
}

#[test]
fn a_source_declaration_of_a_spliced_name_is_still_e0428() {
    let rendered = lower(&format!(
        "unsafe extern fn __aelys_vec_pop(p: i64) -> i64{MAIN}"
    ))
    .expect_err("E5");
    assert!(rendered.contains("E0428"), "E5: {rendered}");
}

#[test]
fn a_collecting_program_still_compiles_and_runs_under_rc_cycles() {
    let source = "fn main() -> i64 {\n\
                  \u{20}   let mut v = Vec::new()\n\
                  \u{20}   Vec::push(v, 7)\n\
                  \u{20}   Vec::push(v, 9)\n\
                  \u{20}   __aelys_collect()\n\
                  \u{20}   return Vec::len(v)\n\
                  }\n";
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("e6.aelys");
    fs::write(&path, source).expect("write");
    for (name, opt) in LEVELS {
        compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::RcCycles)
            .unwrap_or_else(|err| panic!("E6 {name}: {err}"));
        let code = Command::new(dir.path().join("e6"))
            .status()
            .expect("run")
            .code()
            .unwrap_or(-1);
        assert_eq!(code, 2, "E6 {name}: the vec survives the collection");
    }
}

fn repo_root() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR")).parent().expect("root")
}

fn calls_rs() -> String {
    fs::read_to_string(repo_root().join("codegen/src/lowering/calls.rs")).expect("read calls.rs")
}

// a name stops at the first character a rust identifier cannot carry, so a format! prefix keeps its own shape
fn runtime_literal_head(rest: &str) -> String {
    let end = rest
        .find(|c: char| !c.is_ascii_lowercase() && !c.is_ascii_digit() && c != '_')
        .unwrap_or(rest.len());
    format!("__aelys_{}", &rest[..end])
}

fn runtime_literals(text: &str) -> BTreeSet<String> {
    text.split("\"__aelys_")
        .skip(1)
        .map(runtime_literal_head)
        .collect()
}

fn runtime_literals_after(text: &str, marker: &str) -> BTreeSet<String> {
    let mut found = BTreeSet::new();
    for chunk in text.split(marker).skip(1) {
        let Some(rest) = chunk.strip_prefix("\"__aelys_") else {
            continue;
        };
        found.insert(runtime_literal_head(rest));
    }
    found
}

fn ad_hoc_list() -> BTreeSet<String> {
    let calls = calls_rs();
    let body = calls
        .split_once("pub(crate) const AD_HOC_RUNTIME_SYMBOLS: &[&str] = &[")
        .expect("the list")
        .1
        .split_once("];")
        .expect("the list ends")
        .0
        .to_string();
    runtime_literals_after(&body, "    ")
}

fn admitted_runtime_symbols() -> BTreeSet<String> {
    let mut admitted = ad_hoc_list();
    for name in BOOTSTRAP_BUILTIN_SYMBOLS {
        if name.starts_with("__aelys_") {
            admitted.insert((*name).to_string());
        }
    }
    admitted
}

fn rust_sources(dir: &Path, out: &mut Vec<String>) {
    for entry in fs::read_dir(dir).expect("read_dir").flatten() {
        let path = entry.path();
        if path.is_dir() {
            rust_sources(&path, out);
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(fs::read_to_string(&path).expect("read"));
        }
    }
}

// generate_call answers for these before resolve_callee sees them, so the guard never needs them
fn intercepted_in_generate_call() -> BTreeSet<String> {
    let calls = calls_rs();
    let generate = calls
        .split_once("fn generate_call(")
        .expect("generate_call")
        .1;
    let body = generate
        .split_once("fn resolve_callee(")
        .expect("resolve_callee follows")
        .0;
    runtime_literals_after(body, "name == ")
}

const GUARD_CARVE_OUTS: [&str; 4] = [
    "__aelys_main",
    "__aelys_user_main",
    "__aelys_global_get_",
    "__aelys_global_set_",
];

#[test]
fn every_runtime_name_air_can_emit_is_intercepted_or_admitted() {
    let mut sources = Vec::new();
    rust_sources(&repo_root().join("air/src"), &mut sources);
    assert!(sources.len() > 10, "E7: air/src did not get read");
    let mut emitted: BTreeSet<String> = BTreeSet::new();
    for text in &sources {
        emitted.extend(runtime_literals(text));
    }
    assert!(
        emitted.contains("__aelys_vec_retain"),
        "E7: the positive control, the name stage 4 first missed, must be seen"
    );

    let intercepted = intercepted_in_generate_call();
    let admitted = admitted_runtime_symbols();
    let unreachable: Vec<&String> = emitted
        .iter()
        .filter(|name| {
            !GUARD_CARVE_OUTS.contains(&name.as_str())
                && !intercepted.contains(*name)
                && !admitted.contains(*name)
        })
        .collect();
    assert!(
        unreachable.is_empty(),
        "E7: air emits these runtime names and the backend refuses them: {unreachable:?}"
    );
}

// these two never survive to the link, air lowers the loop and the range before codegen guesses
const AD_HOC_NAMES_NOT_IN_THE_ARCHIVE: [&str; 2] = ["__aelys_len", "__aelys_range"];

fn nm_defined_runtime_symbols(archive: &Path) -> Option<BTreeSet<String>> {
    let output = Command::new("nm")
        .arg("--extern-only")
        .arg("--defined-only")
        .arg(archive)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(
        String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let fields: Vec<&str> = line.split_whitespace().collect();
                (fields.len() == 3 && fields[2].starts_with("__aelys_"))
                    .then(|| fields[2].to_string())
            })
            .collect(),
    )
}

#[test]
fn the_admitted_runtime_set_is_pinned_and_its_linked_half_is_what_nm_defines() {
    let admitted = admitted_runtime_symbols();
    let pinned: BTreeSet<String> = [
        "__aelys_collect",
        "__aelys_len",
        "__aelys_range",
        "__aelys_vec_release",
        "__aelys_vec_retain",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    assert_eq!(
        admitted, pinned,
        "E8: the guard admits a different set than the one this row measured against nm"
    );

    for variant in [
        RuntimeVariant::Leak,
        RuntimeVariant::Rc,
        RuntimeVariant::RcCycles,
    ] {
        let archive = resolve_aelys_core_lib(variant)
            .unwrap_or_else(|err| panic!("E8: the runtime archive must exist: {err}"));
        let Some(defined) = nm_defined_runtime_symbols(&archive) else {
            panic!(
                "E8: nm from binutils must be on PATH, this row is the only thing that keeps the \
                 admitted set from naming a symbol no runtime defines, so it fails rather than skips"
            );
        };
        for name in &admitted {
            if AD_HOC_NAMES_NOT_IN_THE_ARCHIVE.contains(&name.as_str()) {
                assert!(
                    !defined.contains(name),
                    "E8: {name} is in the archive after all, it belongs to the linked half"
                );
                continue;
            }
            assert!(
                defined.contains(name),
                "E8: the guard admits {name}, which {} does not define",
                archive.display()
            );
        }
    }
}

#[test]
fn an_extern_main_over_a_source_main_answers_e0301_and_the_registry_says_so() {
    let rendered = lower("unsafe extern fn main(x: i64) -> i64\n\nfn main() -> i64 { return 0 }\n")
        .expect_err("E9");
    assert!(
        rendered.contains("E0301") && !rendered.contains("E0613"),
        "E9: the duplicate check answers before the reservation, found:\n{rendered}"
    );
    let info = aelys_common::registry::lookup("E0613").expect("E0613 must be registered");
    assert!(
        info.explanation.contains("E0301"),
        "E9: E0613 must say which check answers first, got:\n{}",
        info.explanation
    );
}

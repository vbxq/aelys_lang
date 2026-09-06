use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::Command;

use aelys_air::bir::build::for_each_fn_decl;
use aelys_air::symbols::{
    RUNTIME_DEFINED_SYMBOLS, RUNTIME_IMPORTED_SYMBOLS, RUNTIME_RESERVED_SYMBOLS, duplicate_symbols,
    reserved_runtime_symbols, reserved_user_names, symbol_for_source_name,
};
use aelys_air::{
    AirBlock, AirFunction, AirParam, AirProgram, AirTerminator, AirType, BlockId, CallingConv,
    FunctionAttribs, FunctionId, GcMode, InlineHint, LocalId,
};
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_driver::{RuntimeVariant, compile_file_with_llvm, lower_file_to_air, resolve_aelys_core_lib};
use aelys_opt::OptimizationLevel;
use aelys_sema::{InferType, TypeTable, TypedFunction, TypedProgram, TypedStmt, TypedStmtKind};
use aelys_syntax::{Source, Span as SyntaxSpan};
use tempfile::tempdir;

fn empty_program() -> AirProgram {
    AirProgram {
        functions: Vec::new(),
        structs: Vec::new(),
        enums: Vec::new(),
        globals: Vec::new(),
        source_files: Vec::new(),
        mono_instances: Vec::new(),
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    }
}

fn attribs() -> FunctionAttribs {
    FunctionAttribs {
        inline: InlineHint::Default,
        no_gc: false,
        no_unwind: false,
        cold: false,
    }
}

fn function(id: u32, name: &str, params: &[AirType], is_extern: bool) -> AirFunction {
    let blocks = if is_extern {
        Vec::new()
    } else {
        vec![AirBlock {
            id: BlockId(0),
            stmts: Vec::new(),
            terminator: AirTerminator::Return(None),
        }]
    };
    AirFunction {
        id: FunctionId(id),
        name: name.to_string(),
        gc_mode: GcMode::Managed,
        type_params: Vec::new(),
        params: params
            .iter()
            .enumerate()
            .map(|(i, ty)| AirParam {
                id: LocalId(i as u32),
                ty: ty.clone(),
                name: format!("p{i}"),
                span: None,
            })
            .collect(),
        ret_ty: AirType::I64,
        locals: Vec::new(),
        blocks,
        is_extern,
        calling_conv: if is_extern {
            CallingConv::C
        } else {
            CallingConv::Aelys
        },
        attributes: attribs(),
        span: None,
    }
}

// the surface cannot mark a function extern yet, so only a hand built air reaches the has_extern arm
fn program(functions: Vec<AirFunction>) -> AirProgram {
    let mut air = empty_program();
    air.functions = functions;
    air
}

fn verdicts(functions: Vec<AirFunction>) -> Vec<String> {
    duplicate_symbols(&program(functions))
        .into_iter()
        .map(|dup| dup.symbol)
        .collect()
}

#[test]
fn duplicate_symbols_answers_only_for_a_group_that_holds_a_definition() {
    let verdicts = verdicts(vec![
        function(0, "f", &[AirType::I64], true),
        function(1, "f", &[AirType::I64, AirType::I64], true),
    ]);
    assert_eq!(
        verdicts,
        Vec::<String>::new(),
        "this function answers for a symbol the program defines; a group made only of \
         declarations holds no body it could be resolved against"
    );
}

#[test]
fn an_extern_over_a_definition_is_a_conflict() {
    let air = program(vec![
        function(0, "f", &[AirType::I64], false),
        function(1, "f", &[AirType::I64, AirType::I64], true),
    ]);
    let found = duplicate_symbols(&air);
    assert_eq!(
        found
            .iter()
            .map(|dup| dup.symbol.clone())
            .collect::<Vec<_>>(),
        vec!["f".to_string()],
        "a group holding one definition and one declaration of the same symbol is a conflict"
    );
    assert!(
        found[0].has_extern,
        "the group must carry the mark that routes it to E0612 rather than E0427"
    );
    assert_eq!(
        found[0].spans.len(),
        2,
        "both sites are carried to the render"
    );
}

#[test]
fn two_definitions_of_one_symbol_stay_on_the_older_code() {
    let air = program(vec![
        function(0, "g", &[AirType::I64], false),
        function(1, "g", &[AirType::I64], false),
    ]);
    let found = duplicate_symbols(&air);
    assert_eq!(found.len(), 1, "two bodies on one symbol stay a conflict");
    assert!(
        !found[0].has_extern,
        "no declaration is involved, so the group keeps the E0427 route"
    );
}

#[test]
fn e0612_names_the_symbol_and_both_sites() {
    let source = Source::new(
        "<unit>",
        "fn f(a: i64) -> i64 { return a }\nfn main() -> i64 { return f(1) }\n",
    );
    let mut diag = CompileError::new(
        CompileErrorKind::ConflictingExternalSymbol {
            symbol: "f".to_string(),
        },
        SyntaxSpan::new(3, 4, 1, 4),
        source.clone(),
    )
    .to_diagnostic();
    diag.add_secondary_label(
        source,
        SyntaxSpan::new(36, 37, 2, 4),
        Some("and claimed again here".to_string()),
    );
    let rendered = diag.to_string();
    assert!(rendered.contains("E0612"), "got:\n{rendered}");
    assert!(rendered.contains("'f'"), "got:\n{rendered}");
    assert!(
        rendered.contains("this symbol is claimed here")
            && rendered.contains("and claimed again here"),
        "both sites must be drawn, got:\n{rendered}"
    );
    let info = aelys_common::registry::lookup("E0612").expect("E0612 must be registered");
    assert!(!info.explanation.is_empty(), "E0612 must explain itself");
}

const BIG_ALLOC_MAIN: &str = r#"fn main() -> i64 {
    let mut s: string = ""
    for i in 0..20000 {
        s = s + "x"
    }
    println("{s.len}")
    return 7
}
"#;

const MALLOC_DEFINITION: &str = "fn malloc(n: i64) -> i64 { return 0 }\n";

struct Run {
    stdout: String,
    status: i32,
}

// exitstatus::code() is none when a signal kills the child, so the row reads `$?` from a shell
fn compile_and_run(dir: &Path, name: &str, source: &str) -> Run {
    let source_path = dir.join(format!("{name}.aelys"));
    fs::write(&source_path, source).expect("source should be written");
    compile_file_with_llvm(&source_path, OptimizationLevel::Standard, false)
        .expect("llvm backend compilation should succeed");
    let exe_path = source_path.with_extension("");
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("{} 2>/dev/null; echo $?", exe_path.display()))
        .output()
        .expect("shell should run the compiled executable");
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let mut lines: Vec<&str> = text.lines().collect();
    let status = lines
        .pop()
        .expect("the shell always echoes a status")
        .trim()
        .parse::<i32>()
        .expect("a shell status is an integer below 256");
    Run {
        stdout: lines.join("\n"),
        status,
    }
}

#[test]
fn a_definition_named_malloc_is_rejected_and_its_control_still_runs() {
    let dir = tempdir().expect("tempdir should be created");

    let plain = compile_and_run(dir.path(), "plain", BIG_ALLOC_MAIN);
    assert_eq!(plain.status, 7, "the control program returns its own value");
    assert_eq!(
        plain.stdout, "20000",
        "the control program builds its string"
    );

    let source_path = dir.path().join("hijack.aelys");
    fs::write(&source_path, format!("{MALLOC_DEFINITION}{BIG_ALLOC_MAIN}"))
        .expect("source should be written");
    let rendered = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, false)
        .expect_err("a definition named `malloc` must not reach the linker")
        .to_string();
    assert!(
        rendered.contains("E0613") && rendered.contains("'malloc'"),
        "the rejection must name the symbol, got:\n{rendered}"
    );
    assert!(
        !source_path.with_extension("").is_file(),
        "no executable may be produced for a rejected program"
    );
}

#[test]
fn an_aelys_main_never_claims_the_runtime_entry() {
    let claimed = symbol_for_source_name("main");
    assert_eq!(
        claimed, "__aelys_main",
        "the rule reads the emitted symbol, and `main` is renamed before it reaches one"
    );
    assert!(
        !RUNTIME_RESERVED_SYMBOLS.contains(&claimed.as_str()),
        "an ordinary `fn main` must stay legal"
    );
    assert!(
        RUNTIME_RESERVED_SYMBOLS.contains(&"main"),
        "the C entry the runtime defines is still reserved, for whoever claims it directly"
    );
}

#[test]
fn e0613_names_the_symbol_and_the_site() {
    let source = Source::new("<unit>", "fn free(p: i64) -> i64 { return p }\n");
    let rendered = CompileError::new(
        CompileErrorKind::ReservedRuntimeSymbol {
            symbol: "free".to_string(),
        },
        SyntaxSpan::new(3, 7, 1, 4),
        source,
    )
    .to_diagnostic()
    .to_string();
    assert!(rendered.contains("E0613"), "got:\n{rendered}");
    assert!(rendered.contains("'free'"), "got:\n{rendered}");
    assert!(
        rendered.contains("this name belongs to the runtime"),
        "the site must be drawn, got:\n{rendered}"
    );
    let info = aelys_common::registry::lookup("E0613").expect("E0613 must be registered");
    assert!(!info.explanation.is_empty(), "E0613 must explain itself");
}

// adding a runtimevariant breaks this match, which is the point: the set is per-archive
fn variant_suffix(variant: RuntimeVariant) -> &'static str {
    match variant {
        RuntimeVariant::Leak => "leak",
        RuntimeVariant::Rc => "rc",
        RuntimeVariant::RcCycles => "rc-cycles",
    }
}

const ALL_RUNTIME_VARIANTS: [RuntimeVariant; 3] = [
    RuntimeVariant::Leak,
    RuntimeVariant::Rc,
    RuntimeVariant::RcCycles,
];

const NM_DEFINED_TYPES: [&str; 9] = ["A", "B", "D", "G", "R", "S", "T", "V", "W"];

fn nm_defined(archive: &Path) -> Option<Vec<String>> {
    let output = Command::new("nm")
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
                (fields.len() == 3 && NM_DEFINED_TYPES.contains(&fields[1]))
                    .then(|| fields[2].to_string())
            })
            .collect(),
    )
}

fn nm_undefined(archive: &Path) -> Vec<String> {
    let output = Command::new("nm")
        .arg("-u")
        .arg(archive)
        .output()
        .expect("nm ran once, it runs twice");
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .filter_map(|line| {
            let fields: Vec<&str> = line.split_whitespace().collect();
            (fields.len() == 2 && fields[0] == "U").then(|| fields[1].to_string())
        })
        .collect()
}

#[test]
fn the_reserved_set_is_what_nm_says_about_every_runtime_variant() {
    let mut measured: BTreeSet<String> = BTreeSet::new();
    for variant in ALL_RUNTIME_VARIANTS {
        let archive = resolve_aelys_core_lib(variant).unwrap_or_else(|err| {
            panic!(
                "the {} runtime archive must exist: {err}",
                variant_suffix(variant)
            )
        });
        let Some(defined) = nm_defined(&archive) else {
            panic!(
                "nm from binutils must be on PATH: this row is the only thing that keeps \
                 RUNTIME_RESERVED_SYMBOLS from going stale, so it fails rather than skips"
            );
        };
        for symbol in defined.into_iter().chain(nm_undefined(&archive)) {
            if symbol.starts_with("__") || symbol == "_GLOBAL_OFFSET_TABLE_" {
                continue;
            }
            measured.insert(symbol);
        }
    }
    let pinned: BTreeSet<String> = RUNTIME_RESERVED_SYMBOLS
        .iter()
        .map(|s| s.to_string())
        .collect();
    assert_eq!(
        measured, pinned,
        "the reserved set is derived from the runtime archives, not written by hand; regenerate \
         RUNTIME_RESERVED_SYMBOLS from this row's measurement"
    );
}

fn pinned(names: &[&str]) -> BTreeSet<String> {
    names.iter().map(|s| s.to_string()).collect()
}

#[test]
fn each_half_of_the_reserved_set_is_what_nm_says_about_it() {
    let mut measured_defined: BTreeSet<String> = BTreeSet::new();
    let mut measured_undefined: BTreeSet<String> = BTreeSet::new();
    for variant in ALL_RUNTIME_VARIANTS {
        let archive = resolve_aelys_core_lib(variant).unwrap_or_else(|err| {
            panic!(
                "the {} runtime archive must exist: {err}",
                variant_suffix(variant)
            )
        });
        let Some(defined) = nm_defined(&archive) else {
            panic!(
                "nm from binutils must be on PATH: this row is the only thing that keeps the \
                 split from going stale, so it fails rather than skips"
            );
        };
        for symbol in defined {
            if symbol.starts_with("__") || symbol == "_GLOBAL_OFFSET_TABLE_" {
                continue;
            }
            measured_defined.insert(symbol);
        }
        for symbol in nm_undefined(&archive) {
            if symbol.starts_with("__") || symbol == "_GLOBAL_OFFSET_TABLE_" {
                continue;
            }
            measured_undefined.insert(symbol);
        }
    }

    assert_eq!(
        measured_defined.len(),
        5,
        "the runtime defines five symbols of its own, got: {measured_defined:?}"
    );
    assert_eq!(
        measured_defined,
        pinned(RUNTIME_DEFINED_SYMBOLS),
        "the defined half is what a declaration may never name, so it is derived and not written \
         by hand"
    );

    assert_eq!(
        measured_undefined.len(),
        19,
        "nm reports nineteen undefined symbols, got: {measured_undefined:?}"
    );
    let imported_only: BTreeSet<String> = measured_undefined
        .difference(&measured_defined)
        .cloned()
        .collect();
    assert_eq!(
        imported_only.len(),
        16,
        "sixteen of the nineteen are imported and never defined, got: {imported_only:?}"
    );
    assert_eq!(
        imported_only,
        pinned(RUNTIME_IMPORTED_SYMBOLS),
        "the imported half is derived from the archives too"
    );

    let union: BTreeSet<String> = measured_defined.union(&measured_undefined).cloned().collect();
    assert_eq!(
        union,
        pinned(RUNTIME_RESERVED_SYMBOLS),
        "the union is what a definition is still measured against"
    );
    assert_eq!(
        RUNTIME_RESERVED_SYMBOLS.len(),
        21,
        "the union counts twenty one entries"
    );
}

#[test]
fn f3_36_a_declaration_is_measured_against_the_defined_half_and_a_body_against_the_union() {
    let declarations: Vec<AirFunction> = RUNTIME_RESERVED_SYMBOLS
        .iter()
        .enumerate()
        .map(|(i, name)| function(i as u32, name, &[], true))
        .collect();
    let claimed: BTreeSet<String> = reserved_runtime_symbols(&program(declarations), &[])
        .into_iter()
        .map(|found| found.symbol)
        .collect();
    assert_eq!(
        claimed,
        pinned(RUNTIME_DEFINED_SYMBOLS),
        "a declaration claims no symbol, so only the five the runtime defines itself are closed \
         to it"
    );

    let bodies: Vec<AirFunction> = RUNTIME_RESERVED_SYMBOLS
        .iter()
        .enumerate()
        .map(|(i, name)| function(i as u32, name, &[], false))
        .collect();
    let claimed: BTreeSet<String> = reserved_runtime_symbols(&program(bodies), &[])
        .into_iter()
        .map(|found| found.symbol)
        .collect();
    let mut expected = pinned(RUNTIME_RESERVED_SYMBOLS);
    expected.remove("main");
    assert_eq!(
        claimed, expected,
        "a body is still measured against the whole union, `main` excepted because it is renamed \
         before it reaches a symbol"
    );
}

// unreachable from source: `unsafe extern fn println` is stopped by in inference, -50
#[test]
fn f3_37_a_declaration_of_a_bootstrap_builtin_is_claimed_too() {
    let claimed: Vec<String> = reserved_runtime_symbols(
        &program(vec![function(0, "println", &[], true)]),
        &["print", "println", "__aelys_collect"],
    )
    .into_iter()
    .map(|found| found.symbol)
    .collect();
    assert_eq!(
        claimed,
        vec!["println".to_string()],
        "the names the driver injects as globals are closed to a declaration as well"
    );
}

const EXTERN_MALLOC: &str = "unsafe extern fn malloc(n: i64) -> i64

fn main() -> i64 {
    let p: i64 = unsafe { malloc(16) }
    if p == 0 {
        return 1
    }
    return 23
}
";

// gates the call site on , so the old bare spelling is kept as a rejected twin
#[test]
fn f3_38_bis_the_old_bare_spelling_of_the_malloc_call_is_now_e0617() {
    let bare = "unsafe extern fn malloc(n: i64) -> i64\n\nfn main() -> i64 {\n    let p: i64 = malloc(16)\n    return p\n}\n";
    let rendered = match aelys_driver::compile_to_typed_ast(bare) {
        Ok(_) => panic!("F3-38-bis: the bare call MUST be rejected\n{bare}"),
        Err(rendered) => rendered.to_string(),
    };
    assert!(
        rendered.contains("E0617"),
        "F3-38-bis: the rejection MUST be E0617\nrendered:\n{rendered}"
    );
}

#[test]
fn f3_38_a_declaration_named_malloc_is_accepted_and_answers() {
    let dir = tempdir().expect("tempdir should be created");
    let run = compile_and_run(dir.path(), "extmalloc", EXTERN_MALLOC);
    assert_eq!(
        run.status, 23,
        "a declaration diverts no link, so naming a symbol the runtime merely imports is legal"
    );
}

fn rejected(name: &str, source: &str) -> String {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join(format!("{name}.aelys"));
    fs::write(&source_path, source).expect("source should be written");
    compile_file_with_llvm(&source_path, OptimizationLevel::Standard, false)
        .expect_err("this declaration must not reach the linker")
        .to_string()
}

#[test]
fn f3_39_a_declaration_named_main_is_rejected() {
    let rendered = rejected(
        "extmain",
        "unsafe extern fn main() -> i64

fn f() -> i64 {
    return 0
}
",
    );
    assert!(
        rendered.contains("E0613") && rendered.contains("'main'"),
        "the runtime defines the c entry itself, got:
{rendered}"
    );
}

#[test]
fn f3_40_a_declaration_named_after_the_allocator_is_rejected() {
    let rendered = rejected(
        "extimmix",
        "unsafe extern fn aelys_immix_alloc(n: i64) -> i64

fn main() -> i64 {
    return 0
}
",
    );
    assert!(
        rendered.contains("E0613") && rendered.contains("'aelys_immix_alloc'"),
        "calling a symbol the runtime defines reaches the runtime's own code, got:
{rendered}"
    );
}

// this crash is accepted: `stdout` is a data object, the declaration lies about it, and the author signed `unsafe` in full
#[test]
fn f3_41_a_lying_declaration_of_a_data_object_links_and_crashes() {
    let dir = tempdir().expect("tempdir should be created");
    let run = compile_and_run(
        dir.path(),
        "callstdout",
        "unsafe extern fn stdout() -> i64

fn main() -> i64 {
    unsafe { return stdout() }
}
",
    );
    assert!(
        run.status > 128,
        "the compiler does not read c, so it cannot tell a data object from a function; the          signal is the answer, got status {}",
        run.status
    );
}

#[test]
fn f3_42_the_same_declaration_never_called_answers_normally() {
    let dir = tempdir().expect("tempdir should be created");
    let run = compile_and_run(
        dir.path(),
        "declstdout",
        "unsafe extern fn stdout() -> i64

fn main() -> i64 {
    return 11
}
",
    );
    assert_eq!(
        run.status, 11,
        "the rejection is not in the declaration: only the call reaches the object"
    );
}

#[test]
fn f3_43_e0613_explains_that_a_declaration_is_held_to_the_narrower_half() {
    let info = aelys_common::registry::lookup("E0613").expect("E0613 must be registered");
    assert!(
        info.explanation.contains("`unsafe extern` declaration"),
        "the explanation must distinguish a body from a declaration, got:\n{}",
        info.explanation
    );
    assert!(
        info.explanation.contains("it claims no"),
        "and it must say why the declaration is let through, got:\n{}",
        info.explanation
    );
}

const OPT_LEVELS: [OptimizationLevel; 4] = [
    OptimizationLevel::None,
    OptimizationLevel::Basic,
    OptimizationLevel::Standard,
    OptimizationLevel::Aggressive,
];

const UNSAFE_RETURN: &str =
    "fn g(x: i64) -> i64 { unsafe { return x } }\nfn main() -> i64 { return g(7) }\n";

const UNSAFE_TAIL_CALL: &str = "fn h(x: i64) -> i64 { return x }\nfn g(x: i64) -> i64 { unsafe { h(x) } }\nfn main() -> i64 { return g(7) }\n";

const UNSAFE_RETURN_REFERENCE: &str = "fn g(r: &i64) -> &i64 { unsafe { return r } }\nfn main() -> i64 {\n    let x: i64 = 7\n    let p: &i64 = &x\n    return *g(p)\n}\n";

// this row pinned as the verdict; seals the dead tail block, so it now pins the answer
#[test]
fn an_unsafe_tail_return_of_an_i64_reaches_its_answer() {
    let dir = tempdir().expect("tempdir should be created");
    let run = compile_and_run(dir.path(), "ret", UNSAFE_RETURN);
    assert_eq!(
        run.status, 7,
        "the dead tail block is sealed unreachable, so an i64 tail return under `unsafe` runs"
    );
}

// the same dead tail block emits `ret ptr null`, which the verifier accepts when the type is a pointer
#[test]
fn an_unsafe_return_of_a_reference_compiles_and_runs() {
    let dir = tempdir().expect("tempdir should be created");
    let run = compile_and_run(dir.path(), "ref", UNSAFE_RETURN_REFERENCE);
    assert_eq!(
        run.status, 7,
        "a reference returned from inside `unsafe` reaches the answer, so the class is not rejected"
    );
}

#[test]
fn an_unsafe_tail_call_compiles_and_runs_at_every_opt_level() {
    let dir = tempdir().expect("tempdir should be created");
    for (index, opt) in OPT_LEVELS.into_iter().enumerate() {
        let source_path = dir.path().join(format!("tail{index}.aelys"));
        fs::write(&source_path, UNSAFE_TAIL_CALL).expect("source should be written");
        compile_file_with_llvm(&source_path, opt, false)
            .unwrap_or_else(|err| panic!("the control must compile at -O{index}: {err}"));
        let output = Command::new("sh")
            .arg("-c")
            .arg(format!(
                "{} >/dev/null 2>&1; echo $?",
                source_path.with_extension("").display()
            ))
            .output()
            .expect("shell should run the compiled executable");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout).trim(),
            "7",
            "the control returns its own value at -O{index}"
        );
    }
}

fn declaration(name: &str) -> TypedStmt {
    TypedStmt {
        kind: TypedStmtKind::Function(TypedFunction {
            name: name.to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            return_type: InferType::I64,
            body: Vec::new(),
            decorators: Vec::new(),
            is_pub: false,
            declared_nogc: false,
            foreign: None,
            span: SyntaxSpan::new(0, 1, 1, 1),
            captures: Vec::new(),
        }),
        span: SyntaxSpan::new(0, 1, 1, 1),
    }
}

// an ordinary `fn e() {}` has this exact shape, so the row pins the traversal, not a discrimination
#[test]
fn a_body_less_declaration_is_visited_and_the_reserved_prefix_fires_on_it() {
    let program = TypedProgram {
        stmts: vec![declaration("__aelys_probe"), declaration("plain")],
        source: Source::new("<unit>", ""),
        type_table: TypeTable::new(),
    };

    let mut visited: Vec<String> = Vec::new();
    for_each_fn_decl(&program.stmts, &mut |func, _parent| {
        visited.push(func.name.clone());
    });
    assert_eq!(
        visited,
        vec!["__aelys_probe".to_string(), "plain".to_string()],
        "a declaration without a body must still be visited: stage 1 hangs every extern on this"
    );

    let reserved: Vec<String> = reserved_user_names(&program)
        .into_iter()
        .map(|found| found.name)
        .collect();
    assert_eq!(
        reserved,
        vec!["__aelys_probe".to_string()],
        "the reserved prefix must fire on a body-less declaration"
    );
}

type Files = &'static [(&'static str, &'static str)];

fn stage(files: Files) -> tempfile::TempDir {
    let dir = tempdir().expect("tempdir should be created");
    for (name, body) in files {
        let path = dir.path().join(name);
        fs::create_dir_all(path.parent().expect("a staged file has a parent"))
            .expect("fixture directory should be created");
        fs::write(&path, body).expect("fixture should be written");
    }
    dir
}

fn emitted_symbols(root: &Path) -> Vec<String> {
    lower_file_to_air(root, OptimizationLevel::Standard)
        .unwrap_or_else(|err| panic!("the module program must lower: {err}"))
        .functions
        .iter()
        .map(aelys_air::symbols::function_symbol_name)
        .collect()
}

fn run_root(files: Files) -> Run {
    let dir = stage(files);
    let root_path = dir.path().join("root.aelys");
    compile_file_with_llvm(&root_path, OptimizationLevel::Standard, false)
        .unwrap_or_else(|err| panic!("the module program must compile and link:\n{err}"));
    let exe_path = root_path.with_extension("");
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new("sh")
        .arg("-c")
        .arg(format!("{} 2>/dev/null; echo $?", exe_path.display()))
        .output()
        .expect("shell should run the compiled executable");
    let text = String::from_utf8_lossy(&output.stdout).to_string();
    let mut lines: Vec<&str> = text.lines().collect();
    let status = lines
        .pop()
        .expect("the shell always echoes a status")
        .trim()
        .parse::<i32>()
        .expect("a shell status is an integer below 256");
    Run {
        stdout: lines.join("\n"),
        status,
    }
}

const MODULE_MALLOC: Files = &[
    ("m.aelys", "pub fn malloc(n: i64) -> i64 {\n    return n * 2\n}\n"),
    (
        "root.aelys",
        "needs m\n\nfn main() -> i64 {\n    return m.malloc(3)\n}\n",
    ),
];

#[test]
fn a_module_definition_named_malloc_reaches_the_link_qualified_and_runs() {
    let dir = stage(MODULE_MALLOC);
    let symbols = emitted_symbols(&dir.path().join("root.aelys"));
    assert!(
        symbols.iter().any(|s| s == "m.malloc"),
        "the module body claims a dotted symbol, got: {symbols:?}"
    );
    assert!(
        !symbols.iter().any(|s| s == "malloc"),
        "no bare `malloc` may be emitted, got: {symbols:?}"
    );
    assert_eq!(
        run_root(MODULE_MALLOC).status,
        6,
        "the program the guard used to reject must compile, link and answer"
    );
}

#[test]
fn a_root_definition_named_malloc_is_still_rejected() {
    let dir = stage(&[(
        "root.aelys",
        "fn malloc(n: i64) -> i64 {\n    return n * 2\n}\nfn main() -> i64 {\n    return malloc(3)\n}\n",
    )]);
    let rendered = compile_file_with_llvm(
        &dir.path().join("root.aelys"),
        OptimizationLevel::Standard,
        false,
    )
    .expect_err("the root unit emits a bare `malloc`")
    .to_string();
    assert!(
        rendered.contains("E0613") && rendered.contains("'malloc'"),
        "the rejection must name the symbol, got:\n{rendered}"
    );
}

// privateness never reaches the symbol, so the guard has nothing to say about it
#[test]
fn a_private_module_definition_named_malloc_is_accepted() {
    const FILES: Files = &[
        (
            "m.aelys",
            "fn malloc(n: i64) -> i64 {\n    return n * 3\n}\npub fn through(n: i64) -> i64 {\n    return malloc(n)\n}\n",
        ),
        (
            "root.aelys",
            "needs m\n\nfn main() -> i64 {\n    return m.through(3)\n}\n",
        ),
    ];
    assert_eq!(run_root(FILES).status, 9, "a private body is qualified too");
}

#[test]
fn a_nested_module_path_keeps_a_reserved_name_off_the_link() {
    const FILES: Files = &[
        (
            "a/b/c.aelys",
            "pub fn malloc(n: i64) -> i64 {\n    return n * 2\n}\n",
        ),
        (
            "root.aelys",
            "needs a.b.c\n\nfn main() -> i64 {\n    return c.malloc(4)\n}\n",
        ),
    ];
    let dir = stage(FILES);
    let symbols = emitted_symbols(&dir.path().join("root.aelys"));
    assert!(
        symbols.iter().any(|s| s == "a.b.c.malloc"),
        "a nested path qualifies with every segment, got: {symbols:?}"
    );
    assert_eq!(run_root(FILES).status, 8, "the nested program answers");
}

#[test]
fn one_reserved_name_defined_in_two_modules_at_once_is_accepted() {
    const FILES: Files = &[
        ("m.aelys", "pub fn malloc(n: i64) -> i64 {\n    return n * 2\n}\n"),
        ("n.aelys", "pub fn malloc(n: i64) -> i64 {\n    return n + 100\n}\n"),
        (
            "root.aelys",
            "needs m\nneeds n\n\nfn main() -> i64 {\n    return m.malloc(1) + n.malloc(1)\n}\n",
        ),
    ];
    let dir = stage(FILES);
    let symbols = emitted_symbols(&dir.path().join("root.aelys"));
    assert!(
        symbols.iter().any(|s| s == "m.malloc") && symbols.iter().any(|s| s == "n.malloc"),
        "two homonymous module bodies stay two symbols, got: {symbols:?}"
    );
    assert_eq!(run_root(FILES).status, 103, "both bodies answer for themselves");
}

#[test]
fn a_diamond_over_an_aliased_reserved_name_stays_one_qualified_definition() {
    const FILES: Files = &[
        ("base.aelys", "pub fn free(n: i64) -> i64 {\n    return n * 2\n}\n"),
        (
            "left.aelys",
            "needs base as z\npub fn l(n: i64) -> i64 {\n    return z.free(n)\n}\n",
        ),
        (
            "right.aelys",
            "needs base\npub fn r(n: i64) -> i64 {\n    return base.free(n) + 1\n}\n",
        ),
        (
            "root.aelys",
            "needs left\nneeds right\n\nfn main() -> i64 {\n    return left.l(2) + right.r(2)\n}\n",
        ),
    ];
    let dir = stage(FILES);
    let symbols = emitted_symbols(&dir.path().join("root.aelys"));
    assert_eq!(
        symbols.iter().filter(|s| s.as_str() == "base.free").count(),
        1,
        "two importers of one module share one definition, got: {symbols:?}"
    );
    assert_eq!(run_root(FILES).status, 9, "the diamond answers");
}

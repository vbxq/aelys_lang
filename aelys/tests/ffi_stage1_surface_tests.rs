use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::OptimizationLevel;
use aelys_syntax::{Source, StmtKind};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

struct Rejection {
    code: String,
    message: String,
}

fn parse_source(source: &str) -> Result<Vec<aelys_syntax::Stmt>, Vec<Rejection>> {
    let src = Source::new("<row>", source);
    let tokens = match Lexer::with_source(src.clone()).scan() {
        Ok(tokens) => tokens,
        Err(err) => return Err(rejections(err)),
    };
    Parser::new(tokens, src).parse().map_err(rejections)
}

fn rejections(err: aelys_common::error::AelysError) -> Vec<Rejection> {
    use aelys_common::error::AelysError;
    let diags = match err {
        AelysError::Compile(e) => vec![e.to_diagnostic()],
        AelysError::Multiple(diags) => diags,
    };
    diags
        .into_iter()
        .map(|d| Rejection {
            code: d.code.unwrap_or_default(),
            message: d.message,
        })
        .collect()
}

fn accepted_declaration(id: &str, source: &str) -> aelys_syntax::Function {
    let stmts = match parse_source(source) {
        Ok(stmts) => stmts,
        Err(found) => panic!(
            "{id}: MUST be accepted by the surface, rejected with {:?}",
            found
                .iter()
                .map(|r| format!("{} {}", r.code, r.message))
                .collect::<Vec<_>>()
        ),
    };
    assert_eq!(stmts.len(), 1, "{id}: the row declares exactly one item");
    match &stmts[0].kind {
        StmtKind::Function(func) => func.clone(),
        other => panic!("{id}: MUST parse as a function declaration, found {other:?}"),
    }
}

fn foreign_row(id: &str, source: &str) -> aelys_syntax::Function {
    let func = accepted_declaration(id, source);
    assert!(
        func.foreign.is_some(),
        "{id}: the declaration MUST carry the foreign marker"
    );
    assert!(
        func.body.is_empty(),
        "{id}: an external declaration MUST carry no body"
    );
    assert!(!func.is_pub, "{id}: an external declaration is never `pub`");
    func
}

fn rejected_row(id: &str, source: &str, code: &str, needle: &str) {
    let found = match parse_source(source) {
        Ok(_) => panic!("{id}: MUST be rejected, but the surface accepted it"),
        Err(found) => found,
    };
    assert!(
        found.iter().any(|r| r.code == code),
        "{id}: the diagnostic MUST carry {code}, found {:?}",
        found
            .iter()
            .map(|r| format!("{} {}", r.code, r.message))
            .collect::<Vec<_>>()
    );
    assert!(
        found
            .iter()
            .any(|r| r.code == code && r.message.contains(needle)),
        "{id}: the {code} message MUST say {needle:?}, found {:?}",
        found.iter().map(|r| r.message.clone()).collect::<Vec<_>>()
    );
}

const E0614: &str = "E0614";
const E0101: &str = "E0101";

#[test]
fn row_01_a_bare_declaration_is_accepted() {
    let func = foreign_row("F1-1", "unsafe extern fn f()\n");
    assert_eq!(func.name, "f");
    assert!(func.params.is_empty());
    assert!(func.return_type.is_none());
}

#[test]
fn row_02_a_return_type_is_accepted() {
    let func = foreign_row("F1-2", "unsafe extern fn f() -> i64\n");
    assert!(func.return_type.is_some());
}

#[test]
fn row_03_one_parameter_is_accepted() {
    let func = foreign_row("F1-3", "unsafe extern fn f(x: i64)\n");
    assert_eq!(func.params.len(), 1);
}

#[test]
fn row_04_the_nominal_form_is_accepted() {
    let func = foreign_row("F1-4", "unsafe extern fn f(x: i64) -> i64\n");
    assert_eq!(func.params.len(), 1);
    assert!(func.return_type.is_some());
}

#[test]
fn row_05_two_parameters_are_accepted() {
    let func = foreign_row("F1-5", "unsafe extern fn f(x: i64, y: f64) -> bool\n");
    assert_eq!(func.params.len(), 2);
}

#[test]
fn row_06_the_nogc_claim_is_accepted() {
    let func = foreign_row("F1-6", "unsafe extern nogc fn f(x: i64) -> i64\n");
    assert!(func.is_nogc, "F1-6: the `nogc` claim MUST reach the ast");
}

#[test]
fn row_07_a_trailing_comma_is_accepted() {
    let func = foreign_row("F1-7", "unsafe extern fn f(x: i64,)\n");
    assert_eq!(func.params.len(), 1);
}

#[test]
fn row_08_extern_without_unsafe_is_rejected() {
    rejected_row("F1-8", "extern fn f()\n", E0614, "must be `unsafe`");
}

#[test]
fn row_09_extern_nogc_without_unsafe_is_rejected() {
    rejected_row("F1-9", "extern nogc fn f()\n", E0614, "must be `unsafe`");
}

#[test]
fn row_10_a_pub_external_declaration_is_rejected() {
    rejected_row(
        "F1-10",
        "pub unsafe extern fn f()\n",
        E0614,
        "cannot be `pub`",
    );
}

#[test]
fn row_11_pub_extern_without_unsafe_is_rejected() {
    rejected_row("F1-11", "pub extern fn f()\n", E0614, "must be `unsafe`");
}

#[test]
fn row_12_an_empty_body_is_rejected() {
    rejected_row("F1-12", "unsafe extern fn f() { }\n", E0614, "has no body");
}

#[test]
fn row_13_a_body_is_rejected() {
    rejected_row(
        "F1-13",
        "unsafe extern fn f() -> i64 { return 0 }\n",
        E0614,
        "has no body",
    );
}

#[test]
fn row_14_the_cold_decorator_is_rejected() {
    rejected_row(
        "F1-14",
        "@cold\nunsafe extern fn f()\n",
        E0614,
        "carries no decorator",
    );
}

#[test]
fn row_15_the_inline_always_decorator_is_rejected() {
    rejected_row(
        "F1-15",
        "@inline_always\nunsafe extern fn f()\n",
        E0614,
        "carries no decorator",
    );
}

#[test]
fn row_16_a_type_parameter_is_rejected() {
    rejected_row(
        "F1-16",
        "unsafe extern fn f<T>(x: T)\n",
        E0614,
        "takes no type parameter",
    );
}

#[test]
fn row_17_a_nogc_bounded_type_parameter_is_rejected() {
    rejected_row(
        "F1-17",
        "unsafe extern fn f<T: nogc>(x: T)\n",
        E0614,
        "takes no type parameter",
    );
}

#[test]
fn row_18_a_declaration_inside_a_body_is_rejected() {
    rejected_row(
        "F1-18",
        "fn main() -> i64 {\n    unsafe extern fn f()\n    return 0\n}\n",
        E0614,
        "belongs at the top level",
    );
}

#[test]
fn row_19_extern_first_is_the_same_first_violation() {
    rejected_row(
        "F1-19",
        "extern unsafe nogc fn f()\n",
        E0614,
        "must be `unsafe`",
    );
}

#[test]
fn row_20_unsafe_nogc_extern_is_not_captured() {
    rejected_row(
        "F1-20",
        "unsafe nogc extern fn f()\n",
        E0101,
        "expected {, found nogc",
    );
}

#[test]
fn row_21_nogc_first_keeps_its_own_message() {
    rejected_row(
        "F1-21",
        "nogc unsafe extern fn f()\n",
        E0101,
        "expected fn after `nogc`, found unsafe",
    );
}

#[test]
fn row_22_an_explicit_convention_has_no_spelling() {
    rejected_row(
        "F1-22",
        "unsafe extern \"C\" fn f()\n",
        E0101,
        "expected `nogc` or `fn` after `extern`, found \"C\"",
    );
}

#[test]
fn row_23_a_nameless_declaration_is_rejected() {
    rejected_row(
        "F1-23",
        "unsafe extern fn ()\n",
        E0101,
        "expected function name, found (",
    );
}

#[test]
fn row_24_a_parameterless_signature_is_rejected() {
    rejected_row("F1-24", "unsafe extern fn f\n", E0101, "expected (");
}

#[test]
fn row_25_a_truncated_declaration_is_rejected() {
    rejected_row(
        "F1-25",
        "unsafe extern\n",
        E0101,
        "expected `nogc` or `fn` after `extern`",
    );
}

#[test]
fn row_26_unsafe_fn_keeps_the_message_it_had() {
    rejected_row(
        "F1-26",
        "unsafe fn f() -> i64 { return 1 }\n",
        E0101,
        "expected {, found fn",
    );
}

#[test]
fn row_27_a_reserved_prefix_reaches_the_surface() {
    let func = foreign_row("F1-27", "unsafe extern fn __aelys_x()\n");
    assert_eq!(func.name, "__aelys_x");
}

#[test]
fn row_28_the_name_main_reaches_the_surface() {
    let func = foreign_row("F1-28", "unsafe extern fn main()\n");
    assert_eq!(func.name, "main");
}

#[test]
fn row_29_a_runtime_symbol_reaches_the_surface() {
    let func = foreign_row(
        "F1-29",
        "unsafe extern fn aelys_immix_alloc(n: i64) -> i64\n",
    );
    assert_eq!(func.name, "aelys_immix_alloc");
}

#[test]
fn row_30_a_builtin_name_reaches_the_surface() {
    let func = foreign_row("F1-30", "unsafe extern fn println(x: i64)\n");
    assert_eq!(func.name, "println");
}

#[test]
fn row_31_a_libc_symbol_reaches_the_surface() {
    let func = foreign_row("F1-31", "unsafe extern fn malloc(n: i64) -> i64\n");
    assert_eq!(func.name, "malloc");
    let decl = func.foreign.expect("the foreign marker");
    assert_eq!(decl.symbol, "malloc");
    assert!(decl.is_unsafe);
}

#[test]
fn row_32_extern_is_no_longer_a_function_name() {
    rejected_row(
        "F1-32",
        "fn extern() -> i64 { return 1 }\n",
        E0101,
        "expected function name, found extern",
    );
}

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
}

fn runs_to(id: &str, source: &str, exit: i32) {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, source).expect("write fixture");
    if let Err(err) =
        compile_file_with_llvm_variant(&root, OptimizationLevel::None, false, RuntimeVariant::Rc)
    {
        panic!("{id}: the program MUST compile and link\nerror:\n{err}");
    }
    let exe = exe_path_for(&root);
    let out = Command::new(&exe).output().expect("run executable");
    assert_eq!(
        out.status.code(),
        Some(exit),
        "{id}: the answer MUST be {exit}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn row_33_an_unsafe_block_in_a_body_still_runs() {
    runs_to(
        "F1-33",
        "fn main() -> i64 {\n    unsafe { }\n    return 7\n}\n",
        7,
    );
}

#[test]
fn row_34_an_unsafe_block_at_the_top_level_still_runs() {
    runs_to(
        "F1-34",
        "unsafe { }\n\nfn main() -> i64 {\n    return 8\n}\n",
        8,
    );
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn fixtures() -> Vec<PathBuf> {
    let root = repo_root();
    let mut found = Vec::new();
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = match fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.file_name().is_some_and(|n| n == "target") {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "aelys") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn the_fixture_corpus_is_unmoved_by_the_new_keyword() {
    let files = fixtures();
    assert_eq!(files.len(), 450, "the fixture corpus MUST hold 450 files");
    let mut ok = Vec::new();
    let mut rejected = Vec::new();
    for path in &files {
        match aelys_driver::lower_file_to_air(path, OptimizationLevel::None) {
            Ok(_) => ok.push(path.clone()),
            Err(_) => rejected.push(path.clone()),
        }
    }
    assert_eq!(
        (ok.len(), rejected.len()),
        (404, 46),
        "the surface MUST answer 404 accepted and 46 rejected, unchanged by the new keyword\n\
         rejected:\n{}",
        rejected
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    );
}

fn fixtures_spelling(word: &str) -> Vec<String> {
    let mut spelled = Vec::new();
    for path in fixtures() {
        let text = fs::read_to_string(&path).expect("read fixture");
        if text
            .split(|c: char| !c.is_alphanumeric() && c != '_')
            .any(|w| w == word)
        {
            spelled.push(
                path.strip_prefix(repo_root())
                    .unwrap_or(&path)
                    .display()
                    .to_string(),
            );
        }
    }
    spelled
}

#[test]
fn no_fixture_spells_unsafe() {
    assert!(
        fixtures_spelling("unsafe").is_empty(),
        "the fixture sweep is blind to the `unsafe` class, so it cannot stand alone as the guard"
    );
}

#[test]
fn one_fixture_spells_extern_and_it_was_already_rejected() {
    assert_eq!(
        fixtures_spelling("extern"),
        vec!["tests_e2e/evil50_extern_c.aelys".to_string()],
        "the corpus holds exactly one spelling of the new keyword"
    );
    let path = repo_root().join("tests_e2e/evil50_extern_c.aelys");
    let rendered = aelys_driver::lower_file_to_air(&path, OptimizationLevel::None)
        .err()
        .expect("that fixture MUST stay rejected")
        .to_string();
    assert!(
        rendered.contains("expected decorator name, found extern"),
        "the first violation is the decorator on line 2, not the body-less `fn` on line 3:\n{rendered}"
    );
    assert!(
        rendered.contains("evil50_extern_c.aelys:2:2"),
        "the caret MUST land on line 2:\n{rendered}"
    );
}

#[test]
fn the_unsafe_token_is_read_at_three_sites_in_the_parser() {
    let root = repo_root().join("frontend/src/parser");
    let mut sites = 0usize;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir).expect("read parser directory").flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = fs::read_to_string(&path).expect("read parser source");
                sites += text.matches("TokenKind::Unsafe").count();
            }
        }
    }
    assert_eq!(
        sites, 3,
        "`unsafe` is read by `starts_expression`, by `primary`, and by the foreign lookahead"
    );
}

fn typed(id: &str, source: &str) -> aelys_sema::TypedProgram {
    match aelys_driver::compile_to_typed_ast(source) {
        Ok(program) => program,
        Err(err) => panic!("{id}: MUST type-check\nerror:\n{err}"),
    }
}

fn typed_functions(program: &aelys_sema::TypedProgram) -> Vec<aelys_sema::TypedFunction> {
    let mut found = Vec::new();
    aelys_air::bir::build::for_each_fn_decl(&program.stmts, &mut |func, _parent| {
        found.push(func.clone());
    });
    found
}

#[test]
fn the_foreign_marker_reaches_the_typed_ast() {
    let program = typed(
        "F1-35",
        "unsafe extern fn f(x: i64) -> i64\n\nfn main() -> i64 {\n    return 0\n}\n",
    );
    let funcs = typed_functions(&program);
    let f = funcs
        .iter()
        .find(|func| func.name == "f")
        .expect("F1-35: the traversal MUST see the declaration");
    let decl = f
        .foreign
        .as_ref()
        .expect("F1-35: the marker MUST survive to the typed ast");
    assert_eq!(decl.symbol, "f");
    assert!(decl.is_unsafe);
    assert_eq!(f.return_type, aelys_sema::InferType::I64);
    let main = funcs.iter().find(|func| func.name == "main").unwrap();
    assert!(
        main.foreign.is_none(),
        "F1-35: an ordinary body is not foreign"
    );
}

#[test]
fn every_foreign_declaration_reaches_the_typed_ast_without_a_body() {
    let program = typed(
        "F1-35a",
        "unsafe extern fn a()\nunsafe extern fn b() -> i64\nunsafe extern nogc fn c(x: f64) -> bool\n\nfn main() -> i64 {\n    return 0\n}\n",
    );
    for func in typed_functions(&program) {
        if func.foreign.is_some() {
            assert!(
                func.body.is_empty(),
                "F1-35a: `{}` carries the marker and a body",
                func.name
            );
        }
    }
    let funcs = typed_functions(&program);
    let c = funcs.iter().find(|f| f.name == "c").unwrap();
    assert!(c.declared_nogc, "F1-35a: the `nogc` claim survives typing");
    assert_eq!(c.return_type, aelys_sema::InferType::Bool);
}

#[test]
fn an_unannotated_foreign_return_is_void_and_silent() {
    let source = "unsafe extern fn f()\n\nfn main() -> i64 {\n    return 0\n}\n";
    let program = typed("F1-35b", source);
    let funcs = typed_functions(&program);
    let f = funcs.iter().find(|func| func.name == "f").unwrap();
    assert_eq!(
        f.return_type,
        aelys_sema::InferType::Null,
        "F1-35b: an unannotated foreign return is posed, never left to a variable"
    );
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, source).expect("write fixture");
    let air = aelys_driver::lower_file_to_air(&root, OptimizationLevel::None)
        .expect("F1-35b: the bare form MUST lower without a diagnostic");
    let printed = aelys_air::print::print_program(&air);
    assert!(
        printed.contains("fn f() -> void"),
        "F1-35b: the bare form MUST lower to a void return, found:\n{printed}"
    );
    assert!(
        printed.contains("fn f() -> void  [extern]"),
        "F1-35b: the bare form MUST lower to an extern air function, found:\n{printed}"
    );
    compile_file_with_llvm_variant(&root, OptimizationLevel::None, true, RuntimeVariant::Rc)
        .expect("F1-35b: the bare form MUST reach llvm");
    let ir = fs::read_to_string(root.with_extension("ll")).expect("read emitted .ll");
    assert!(
        ir.contains("declare void @f()"),
        "F1-35b: the bare form MUST declare `void @f()`, found:\n{ir}"
    );
    assert!(
        !ir.contains("declare ptr @f("),
        "F1-35b: a null return posed as `ptr` is an abi divergence on the nominal form, \
         found:\n{ir}"
    );
}

#[test]
fn an_ordinary_empty_body_is_still_rejected() {
    let err = aelys_driver::compile_to_typed_ast("fn f() -> i64 {}\n")
        .expect_err("F1-36: an aelys body that returns nothing MUST stay rejected");
    let rendered = err.to_string();
    assert!(
        rendered.contains("E0301"),
        "F1-36: the rejection MUST stay E0301, found:\n{rendered}"
    );
}

#[test]
fn the_typed_function_is_built_at_five_sites() {
    let root = repo_root();
    let mut sites = Vec::new();
    for crate_dir in ["sema/src", "air/src"] {
        let mut stack = vec![root.join(crate_dir)];
        while let Some(dir) = stack.pop() {
            for entry in fs::read_dir(&dir).expect("read crate directory").flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                } else if path.extension().is_some_and(|e| e == "rs") {
                    let text = fs::read_to_string(&path).expect("read source");
                    for line in text.lines() {
                        if line.contains("TypedFunction {")
                            && !line.contains("pub struct")
                            && !line.contains("-> TypedFunction {")
                        {
                            sites.push(format!("{}: {}", path.display(), line.trim()));
                        }
                    }
                }
            }
        }
    }
    assert_eq!(
        sites.len(),
        5,
        "the marker has to be copied at every construction site\n{}",
        sites.join("\n")
    );
}

fn e0607_rendering() -> String {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(
        &root,
        "needs \"GL/glext.h\"\n\nfn main() -> i64 {\n    return 0\n}\n",
    )
    .expect("write fixture");
    compile_file_with_llvm_variant(&root, OptimizationLevel::None, false, RuntimeVariant::Rc)
        .expect_err("a c header target MUST be rejected")
        .to_string()
}

#[test]
fn f3_44_e0607_points_at_the_supported_route() {
    let rendered = e0607_rendering();
    assert!(
        rendered.contains("E0607") && rendered.contains("importing the C header"),
        "F3-44: the verdict and its message stay what they were\nrendered:\n{rendered}"
    );
    assert!(
        rendered.contains("C header import is not implemented yet"),
        "F3-44: the annotation stays what it was\nrendered:\n{rendered}"
    );
    assert!(
        rendered.contains("= help:") && rendered.contains("unsafe extern fn NAME(...) -> T"),
        "F3-44: reading a header is still not implemented, but a route exists and a route is \
         written in a help\nrendered:\n{rendered}"
    );
}

#[test]
fn f3_45_the_e0607_help_names_the_bound_of_the_route_it_offers() {
    let rendered = e0607_rendering();
    let help: String = rendered
        .lines()
        .filter(|line| line.trim_start().starts_with("= help:"))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        help.contains("E0615"),
        "F3-45: a help that sends the reader at a route which will refuse half of the header is \
         a help that lies, so it cites the code that renders the refusal\nhelp:\n{help}"
    );
    assert!(
        help.contains("integers, floats, `bool` or references"),
        "F3-45: and it names the surface itself\nhelp:\n{help}"
    );
}

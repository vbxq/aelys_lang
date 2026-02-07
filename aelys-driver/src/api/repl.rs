use crate::modules::load_modules_for_program;
use aelys_backend::Compiler;
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::{OptimizationLevel, Optimizer};
use aelys_runtime::{VM, Value};
use aelys_sema::TypeInference;
use aelys_syntax::{Source, Span};

const BUILTIN_NAMES: &[&str] = &["alloc", "free", "load", "store", "type"];

// REPL mode - uses Basic opt to keep top-level vars for subsequent inputs
pub fn run_with_vm(vm: &mut VM, source: &str, name: &str) -> Result<Value> {
    run_with_vm_and_opt(vm, source, name, OptimizationLevel::Basic)
}

pub fn run_with_vm_and_opt(
    vm: &mut VM,
    source: &str,
    name: &str,
    opt_level: OptimizationLevel,
) -> Result<Value> {
    vm.clear_frames();

    let src = Source::new(name, source);
    let tokens = Lexer::with_source(src.clone()).scan()?;
    let stmts = Parser::new(tokens, src.clone()).parse()?;

    let has_needs = stmts
        .iter()
        .any(|s| matches!(s.kind, aelys_syntax::StmtKind::Needs(_)));

    let mut module_aliases = vm.repl_module_aliases().clone();
    let mut known_globals = vm.repl_known_globals().clone();
    let mut known_native_globals = vm.repl_known_native_globals().clone();
    let mut symbol_origins = vm.repl_symbol_origins().clone();

    for builtin in BUILTIN_NAMES {
        known_globals.insert(builtin.to_string());
    }

    let main_stmts = if has_needs {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        let repl_path = cwd.join("repl.aelys");

        let imports = load_modules_for_program(&stmts, &repl_path, src.clone(), vm)?;

        module_aliases.extend(imports.module_aliases.iter().cloned());
        vm.add_repl_module_aliases(&imports.module_aliases);

        known_globals.extend(imports.known_globals.iter().cloned());
        vm.add_repl_known_globals(&imports.known_globals);

        known_native_globals.extend(imports.known_native_globals.iter().cloned());
        vm.add_repl_known_native_globals(&imports.known_native_globals);

        symbol_origins.extend(
            imports
                .symbol_origins
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );
        vm.add_repl_symbol_origins(&imports.symbol_origins);

        stmts
            .into_iter()
            .filter(|s| !matches!(s.kind, aelys_syntax::StmtKind::Needs(_)))
            .collect()
    } else {
        stmts
    };

    let typed_program = TypeInference::infer_program_with_imports(
        main_stmts,
        src.clone(),
        module_aliases.clone(),
        known_globals.clone(),
    )
    .map_err(|errors| {
        if let Some(err) = errors.first() {
            AelysError::Compile(CompileError::new(
                CompileErrorKind::TypeInferenceError(format!("{}", err)),
                err.span,
                src.clone(),
            ))
        } else {
            AelysError::Compile(CompileError::new(
                CompileErrorKind::TypeInferenceError("Unknown type error".to_string()),
                Span::dummy(),
                src.clone(),
            ))
        }
    })?;

    let mut optimizer = Optimizer::new(opt_level);
    let typed_program = optimizer.optimize(typed_program);

    let existing_globals = vm.global_mutability().clone();
    let compiler = Compiler::with_modules_and_globals(
        None,
        src.clone(),
        module_aliases,
        known_globals,
        known_native_globals,
        symbol_origins,
        existing_globals,
    );
    let (mut function, mut compile_heap, new_globals) = compiler.compile_typed(&typed_program)?;

    vm.update_global_mutability(new_globals);

    let remap = vm.merge_heap(&mut compile_heap)?;
    function.remap_constants(&remap);

    let global_names: Vec<String> = function.global_layout.names().to_vec();

    let func_ref = vm.alloc_function(function)?;
    let result = vm.execute(func_ref)?;

    vm.sync_globals_to_hashmap(&global_names);

    // track new globals for subsequent REPL inputs
    let new_globals_set: std::collections::HashSet<String> = global_names
        .into_iter()
        .filter(|name| !name.is_empty())
        .collect();
    vm.add_repl_known_globals(&new_globals_set);

    Ok(result)
}

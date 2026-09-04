mod core_lib;
mod diagnostics;
mod link;
mod lower;
mod runtime;

pub use runtime::RuntimeVariant;

use aelys_common::Warning;
use aelys_common::error::{AelysError, CompileErrorKind};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::{OptimizationLevel, Optimizer};
use aelys_syntax::Source;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use diagnostics::{
    backend_diagnostic_error, bir_diagnostics_to_error, duplicate_symbol_errors_to_error,
    fallback_source_span, mono_errors_to_error, multiple_diagnostics, program_anchor_span,
    reserved_name_errors_to_error, sema_errors_to_diagnostics, vec_surface_errors_to_error,
};
use lower::compile_air_with_llvm;

// todo: find a better way, clean this up once we have a proper bootstrap
const BOOTSTRAP_BUILTINS: &[&str] = &["print", "println", "__aelys_collect"];

struct LoweringArtifacts {
    air: aelys_air::AirProgram,
    source: Arc<Source>,
    warnings: Vec<Warning>,
}

pub fn compile_to_typed_ast(source_code: &str) -> Result<aelys_sema::TypedProgram, AelysError> {
    let src = Source::new("<inline>", source_code);
    let tokens = Lexer::with_source(src.clone()).scan()?;
    let stmts = Parser::new(tokens, src.clone()).parse()?;

    let known_globals: HashSet<String> = BOOTSTRAP_BUILTINS.iter().map(|s| s.to_string()).collect();

    let inference = aelys_sema::TypeInference::infer_program_full(
        stmts,
        src.clone(),
        aelys_sema::ModuleImports::default(),
        known_globals,
    )
    .map_err(|errors| sema_errors_to_diagnostics(errors, src.clone()))?;

    let aelys_sema::InferenceResult {
        program,
        deferred_errors,
        ..
    } = inference;
    if !deferred_errors.is_empty() {
        return Err(sema_errors_to_diagnostics(deferred_errors, src));
    }
    Ok(program)
}

pub fn lower_file_to_air(
    path: &Path,
    opt_level: OptimizationLevel,
) -> Result<aelys_air::AirProgram, String> {
    let artifacts =
        lower_file_to_air_with_source(path, opt_level).map_err(|err| err.to_string())?;
    Ok(artifacts.air)
}

/// a post-merge diagnostic names a qualified symbol, and the module path it carries is the one
struct ModuleSources {
    by_path: Vec<(String, Arc<Source>)>,
    root: Arc<Source>,
}

impl ModuleSources {
    fn new(units: &[(String, Arc<Source>)], root: Arc<Source>) -> Self {
        let mut by_path: Vec<(String, Arc<Source>)> = units
            .iter()
            .filter(|(path, _)| !path.is_empty())
            .cloned()
            .collect();
        by_path.sort_by(|a, b| b.0.len().cmp(&a.0.len()));
        ModuleSources { by_path, root }
    }

    fn owner(&self, message: &str) -> Arc<Source> {
        let first = message
            .split("; ")
            .next()
            .and_then(|head| head.split('\n').next())
            .unwrap_or(message);
        let bare = first.replace(aelys_sema::modules::TYPE_HEAD, "");
        for (path, source) in &self.by_path {
            if bare.contains(&format!("`{path}.")) {
                return source.clone();
            }
        }
        self.root.clone()
    }
}

fn qualify_chain(path: &str, steps: Vec<aelys_air::bir::Step>) -> Vec<aelys_air::bir::Step> {
    use aelys_air::bir::StepKind;
    steps
        .into_iter()
        .map(|mut step| {
            if matches!(step.kind, StepKind::Root | StepKind::Callee) && !step.name.contains('.') {
                step.name = aelys_sema::modules::qualify_value(path, &step.name);
            }
            step.span = None;
            step
        })
        .collect()
}

struct CompiledModule {
    air: aelys_air::AirProgram,
    typed: aelys_sema::TypedProgram,
    exports: Arc<aelys_sema::ModuleExports>,
    // its own bodies only, under the qualified names an importer writes
    effects: std::collections::HashMap<String, aelys_air::bir::EffectSet>,
    chains: std::collections::HashMap<String, Vec<aelys_air::bir::Step>>,
    warnings: Vec<Warning>,
}

fn build_imports(
    unit: &crate::modules::ModuleUnit,
    exports: &[Arc<aelys_sema::ModuleExports>],
) -> Result<aelys_sema::ModuleImports, AelysError> {
    use aelys_sema::Lookup;

    let mut imports = aelys_sema::ModuleImports {
        is_importable: !unit.dotted.is_empty(),
        ..Default::default()
    };
    for import in &unit.imports {
        match import {
            crate::modules::ResolvedImport::Namespace { local, target } => {
                imports
                    .namespaces
                    .insert(local.clone(), exports[*target].clone());
            }
            crate::modules::ResolvedImport::Symbols {
                names,
                target,
                span,
            } => {
                let module = &exports[*target];
                for name in names {
                    match module.value(name) {
                        Lookup::Found(item) => {
                            imports.values.insert(name.clone(), item.clone());
                            continue;
                        }
                        Lookup::NotPublic => {
                            return Err(import_error(
                                unit,
                                *span,
                                CompileErrorKind::SymbolNotPublic {
                                    symbol: name.clone(),
                                    module: module.path.clone(),
                                },
                            ));
                        }
                        Lookup::Missing => {}
                    }
                    match module.module_type(name) {
                        Lookup::Found(ty) => {
                            imports.types.insert(name.clone(), ty.clone());
                        }
                        Lookup::NotPublic => {
                            return Err(import_error(
                                unit,
                                *span,
                                CompileErrorKind::SymbolNotPublic {
                                    symbol: name.clone(),
                                    module: module.path.clone(),
                                },
                            ));
                        }
                        Lookup::Missing => {
                            return Err(import_error(
                                unit,
                                *span,
                                CompileErrorKind::SymbolNotFound {
                                    symbol: name.clone(),
                                    module: module.path.clone(),
                                },
                            ));
                        }
                    }
                }
            }
        }
    }
    Ok(imports)
}

fn import_error(
    unit: &crate::modules::ModuleUnit,
    span: aelys_syntax::Span,
    kind: CompileErrorKind,
) -> AelysError {
    AelysError::Compile(aelys_common::error::CompileError::new(
        kind,
        span,
        unit.source.clone(),
    ))
}

fn imported_global_symbols(imports: &aelys_sema::ModuleImports) -> HashSet<String> {
    let mut names = HashSet::new();
    let mut take = |item: &aelys_sema::ModuleValue| {
        if matches!(item.kind, aelys_sema::ItemKind::Global) {
            names.insert(item.qualified.clone());
        }
    };
    for exports in imports.namespaces.values() {
        for item in exports.values.values() {
            take(item);
        }
    }
    for item in imports.values.values() {
        take(item);
    }
    names
}

fn compile_module(
    unit: &crate::modules::ModuleUnit,
    imports: aelys_sema::ModuleImports,
    opt_level: OptimizationLevel,
    compiled: &[aelys_air::AirProgram],
    bir_imports: &aelys_air::bir::Imports,
) -> Result<CompiledModule, AelysError> {
    let src = unit.source.clone();
    let imported = aelys_air::lower::Imported {
        globals: imported_global_symbols(&imports),
        structs: compiled.iter().flat_map(|p| p.structs.clone()).collect(),
        enums: compiled.iter().flat_map(|p| p.enums.clone()).collect(),
        bir: bir_imports.clone(),
    };

    let known_globals: HashSet<String> = BOOTSTRAP_BUILTINS.iter().map(|s| s.to_string()).collect();
    let inference = aelys_sema::TypeInference::infer_program_full(
        unit.stmts.clone(),
        src.clone(),
        imports,
        known_globals,
    )
    .map_err(|errors| sema_errors_to_diagnostics(errors, src.clone()))?;

    let aelys_sema::InferenceResult {
        program,
        warnings,
        deferred_errors,
        ..
    } = inference;
    let reserved = aelys_air::symbols::reserved_user_names(&program);
    let mut collected_errors = Vec::new();
    let mut effects = std::collections::HashMap::new();
    let mut chains = std::collections::HashMap::new();
    let checked = match aelys_air::bir::check_with_imports(program, bir_imports) {
        Ok((checked, published)) => {
            let qualify = |name: &str| aelys_sema::modules::qualify_value(&unit.dotted, name);
            effects = published
                .effects
                .into_iter()
                .map(|(name, set)| (qualify(&name), set))
                .collect();
            chains = published
                .chains
                .into_iter()
                .map(|(name, steps)| (qualify(&name), qualify_chain(&unit.dotted, steps)))
                .collect();
            Some(checked)
        }
        Err(errors) => {
            collected_errors.push(bir_diagnostics_to_error(errors, src.clone()));
            None
        }
    };
    if !deferred_errors.is_empty() {
        collected_errors.push(sema_errors_to_diagnostics(deferred_errors, src.clone()));
    }
    if !reserved.is_empty() {
        collected_errors.push(reserved_name_errors_to_error(reserved, src.clone()));
    }
    if !collected_errors.is_empty() {
        return Err(multiple_diagnostics(collected_errors));
    }
    let checked = match checked {
        Some(checked) => checked,
        None => return Err(multiple_diagnostics(collected_errors)),
    };

    let exports = Arc::new(aelys_sema::modules::collect_exports(
        &unit.dotted,
        checked.program(),
    ));

    let mut optimizer = Optimizer::new(opt_level);
    let typed_program = optimizer.optimize(checked);

    let mut air =
        aelys_air::lower::try_lower_with_imports(&typed_program, imported).map_err(|failure| {
            match failure {
                aelys_air::lower::LowerFailure::Borrow(diags) => {
                    bir_diagnostics_to_error(diags, src.clone())
                }
                aelys_air::lower::LowerFailure::Lowering(errors) => {
                    let message = if errors.is_empty() {
                        "AIR lowering failed with an unknown error".to_string()
                    } else {
                        errors
                            .iter()
                            .enumerate()
                            .map(|(i, e)| format!("{}. {}", i + 1, e))
                            .collect::<Vec<_>>()
                            .join("\n")
                    };
                    backend_diagnostic_error(
                        src.clone(),
                        fallback_source_span(src.as_ref()),
                        "air-lowering",
                        message,
                        None,
                        None,
                    )
                }
            }
        })?;

    // mono is function-destroying as well as function-creating: two block-nested `fn g<t>` in
    let duplicates = aelys_air::symbols::duplicate_symbols(&air);
    if !duplicates.is_empty() {
        return Err(duplicate_symbol_errors_to_error(
            duplicates,
            &typed_program,
            &air,
            src.clone(),
        ));
    }

    aelys_air::modules::qualify(&mut air, &unit.dotted);

    let warnings = warnings
        .into_iter()
        .map(|warning| {
            if warning.source.is_none() {
                warning.with_source(src.clone())
            } else {
                warning
            }
        })
        .collect();

    Ok(CompiledModule {
        air,
        typed: typed_program,
        exports,
        effects,
        chains,
        warnings,
    })
}

fn anchor_in(
    air: &aelys_air::AirProgram,
    source: &Source,
    root: &Arc<Source>,
) -> aelys_syntax::Span {
    if std::ptr::eq(source, root.as_ref()) {
        return program_anchor_span(air, source);
    }
    fallback_source_span(source)
}

fn lower_file_to_air_with_source(
    path: &Path,
    opt_level: OptimizationLevel,
) -> Result<LoweringArtifacts, AelysError> {
    let graph = crate::modules::discover(path).map_err(|failure| failure.error)?;

    let mut exports: Vec<Arc<aelys_sema::ModuleExports>> = Vec::new();
    let mut programs: Vec<aelys_air::AirProgram> = Vec::new();
    let mut warnings: Vec<Warning> = Vec::new();
    let mut root_typed: Option<aelys_sema::TypedProgram> = None;
    let src = graph.root().source.clone();
    let sources = ModuleSources::new(
        &graph
            .units
            .iter()
            .map(|unit| (unit.dotted.clone(), unit.source.clone()))
            .collect::<Vec<_>>(),
        src.clone(),
    );

    let mut bir_imports = aelys_air::bir::Imports::default();
    for unit in &graph.units {
        let imports = build_imports(unit, &exports)?;
        let compiled = compile_module(unit, imports, opt_level, &programs, &bir_imports)?;
        for (name, set) in &compiled.effects {
            bir_imports.effects.insert(name.clone(), *set);
        }
        for (name, steps) in &compiled.chains {
            bir_imports.chains.insert(name.clone(), steps.clone());
        }
        exports.push(compiled.exports);
        programs.push(compiled.air);
        warnings.extend(compiled.warnings);
        root_typed = Some(compiled.typed);
    }

    let typed_program = root_typed.expect("a graph always holds its root");

    let air = aelys_air::modules::merge(programs);

    let duplicates = aelys_air::symbols::duplicate_symbols(&air);
    if !duplicates.is_empty() {
        return Err(duplicate_symbol_errors_to_error(
            duplicates,
            &typed_program,
            &air,
            src.clone(),
        ));
    }

    let mut air = aelys_air::mono::monomorphize(air).map_err(|errors| {
        mono_errors_to_error(errors, fallback_source_span(src.as_ref()), src.clone())
    })?;
    // mono joins the name and its type arguments with `_`, so `f<a_b>` and `f_a<b>` land on one
    let duplicates = aelys_air::symbols::duplicate_symbols(&air);
    if !duplicates.is_empty() {
        return Err(duplicate_symbol_errors_to_error(
            duplicates,
            &typed_program,
            &air,
            src.clone(),
        ));
    }
    if let Err(errors) = aelys_air::passes::vec_surface::check_vec_surface(&air) {
        return Err(vec_surface_errors_to_error(errors, &air, src.clone()));
    }
    let layout_errors = aelys_air::layout::compute_layouts(&mut air);
    if !layout_errors.is_empty() {
        let message = layout_errors.join("; ");
        let owner = sources.owner(&message);
        return Err(backend_diagnostic_error(
            owner.clone(),
            anchor_in(&air, owner.as_ref(), &src),
            "air-layout",
            message,
            None,
            None,
        ));
    }

    air.rc_type_table = aelys_air::rc_types::collect_rc_types(&air).map_err(|e| {
        let message = e.to_string();
        let owner = sources.owner(&message);
        backend_diagnostic_error(
            owner.clone(),
            anchor_in(&air, owner.as_ref(), &src),
            "air-rc-types",
            message,
            None,
            None,
        )
    })?;
    aelys_air::passes::copy_elim::eliminate_copies(&mut air);
    aelys_air::passes::dead_locals::eliminate_dead_locals(&mut air);

    // only to keep the -o0 baseline byte-identical, never for soundness
    if opt_level >= OptimizationLevel::Basic {
        aelys_air::passes::rc_elision::eliminate_redundant_rc(&mut air);
    }

    if let Err(validation_errors) = aelys_air::passes::validate::validate_air(&air) {
        let message = validation_errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let owner = sources.owner(&message);
        return Err(backend_diagnostic_error(
            owner.clone(),
            anchor_in(&air, owner.as_ref(), &src),
            "air-validation",
            message,
            None,
            None,
        ));
    }

    Ok(LoweringArtifacts {
        air,
        source: src,
        warnings,
    })
}

/// that can execute an air shape the surface language cannot yet produce, which is exactly
pub fn compile_air_program_to_executable(
    path: &Path,
    air: &aelys_air::AirProgram,
    opt_level: OptimizationLevel,
    runtime: RuntimeVariant,
) -> Result<(), AelysError> {
    let name = path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("<air>")
        .to_string();
    let src = Source::new(&name, "");
    compile_air_with_llvm(path, air, opt_level, false, runtime, src)
}

pub fn compile_file_with_llvm(
    path: &Path,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
) -> Result<(), AelysError> {
    compile_file_with_llvm_variant(path, opt_level, emit_llvm_ir, RuntimeVariant::default())
}

pub fn compile_file_with_llvm_variant(
    path: &Path,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
    runtime: RuntimeVariant,
) -> Result<(), AelysError> {
    let artifacts = lower_file_to_air_with_source(path, opt_level)?;
    compile_air_with_llvm(
        path,
        &artifacts.air,
        opt_level,
        emit_llvm_ir,
        runtime,
        artifacts.source,
    )
}

pub fn compile_file_with_llvm_with_warnings(
    path: &Path,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
    runtime: RuntimeVariant,
) -> Result<Vec<Warning>, AelysError> {
    let artifacts = lower_file_to_air_with_source(path, opt_level)?;
    compile_air_with_llvm(
        path,
        &artifacts.air,
        opt_level,
        emit_llvm_ir,
        runtime,
        artifacts.source,
    )?;
    Ok(artifacts.warnings)
}

fn run_process(program: &str, args: &[String]) -> Result<(), String> {
    run_process_in_dir(program, args, None)
}

fn run_process_in_dir(program: &str, args: &[String], dir: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(path) = dir {
        command.current_dir(path);
    }

    let output = command
        .output()
        .map_err(|err| format!("failed to run `{}`: {}", program, err))?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    Err(format!(
        "`{}` failed with status {:?}\nstdout:\n{}\nstderr:\n{}",
        program,
        output.status.code(),
        stdout.trim(),
        stderr.trim()
    ))
}

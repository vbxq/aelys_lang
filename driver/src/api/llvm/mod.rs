mod core_lib;
mod diagnostics;
mod group;
mod link;
mod lower;
mod runtime;

pub use core_lib::resolve_aelys_core_lib;
pub use link::LinkRequirement;
pub use lower::{CompiledArtifact, executable_path_for, ir_path_for, object_path_for};
pub use runtime::RuntimeVariant;

use crate::SourceOptions;
use aelys_common::Warning;
use aelys_common::error::{AelysError, CompileErrorKind, Fault};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::{OptimizationLevel, Optimizer};
use aelys_syntax::Source;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;

use diagnostics::{
    FaultMessage, all_compiler, backend_diagnostic_error, bir_diagnostics_to_error,
    duplicate_symbol_errors_to_error, fallback_source_span, foreign_clash_errors_to_error,
    foreign_signature_errors_to_error, instance_surface_errors_to_error, join_by_fault,
    lowering_errors_to_error, mono_errors_to_error, multiple_diagnostics,
    optimization_verdict_split, program_anchor_span, reserved_name_errors_to_error,
    runtime_symbol_errors_to_error, sema_errors_to_diagnostics, vec_surface_errors_to_error,
};
use lower::{ObjectPath, compile_air_with_llvm, compile_air_with_llvm_linked};

// todo: find a better way, clean this up once we have a proper bootstrap
use aelys_air::symbols::BOOTSTRAP_BUILTIN_SYMBOLS as BOOTSTRAP_BUILTINS;

const UNOPTIMIZED_LEVEL: &str = "-O0";

struct LoweringArtifacts {
    air: aelys_air::AirProgram,
    source: Arc<Source>,
    warnings: Vec<Warning>,
    // read off the unoptimised air: what the link must resolve is not the optimizer's to decide
    required_externs: Vec<String>,
    files: Vec<(PathBuf, Arc<Source>)>,
}

fn known_globals() -> HashSet<String> {
    BOOTSTRAP_BUILTINS.iter().map(|s| s.to_string()).collect()
}

pub fn compile_to_typed_ast(source_code: &str) -> Result<aelys_sema::TypedProgram, AelysError> {
    let src = Source::new("<inline>", source_code);
    let tokens = Lexer::with_source(src.clone()).scan()?;
    let stmts = Parser::new(tokens, src.clone()).parse()?;

    let inference = aelys_sema::TypeInference::infer_program_full(
        stmts,
        src.clone(),
        aelys_sema::ModuleImports::default(),
        known_globals(),
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
    lower_file_to_air_with_sources(path, opt_level, &SourceOptions::default())
}

pub fn lower_file_to_air_with_sources(
    path: &Path,
    opt_level: OptimizationLevel,
    sources: &SourceOptions,
) -> Result<aelys_air::AirProgram, String> {
    let artifacts =
        lower_file_to_air_with_source(path, opt_level, sources).map_err(|err| err.to_string())?;
    Ok(artifacts.air)
}

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

struct ModuleFront {
    dotted: String,
    source: Arc<Source>,
    globals: HashSet<String>,
    exports: Arc<aelys_sema::ModuleExports>,
    effects: std::collections::HashMap<String, aelys_air::bir::EffectSet>,
    chains: std::collections::HashMap<String, Vec<aelys_air::bir::Step>>,
    externs: std::collections::HashMap<String, aelys_air::bir::ForeignSig>,
    warnings: Vec<Warning>,
    // reached only through a prelude that exports no value, so nothing can call its functions
    functions_unreachable: bool,
}

fn import_chains_and_externs(bir_imports: &mut aelys_air::bir::Imports, front: &ModuleFront) {
    bir_imports.chains.extend(front.chains.clone());
    bir_imports.externs.extend(front.externs.clone());
}

fn build_imports(
    unit: &crate::modules::ModuleUnit,
    unit_index: usize,
    exports: &[Arc<aelys_sema::ModuleExports>],
    tolerant: bool,
) -> Result<aelys_sema::ModuleImports, AelysError> {
    let mut imports = aelys_sema::ModuleImports {
        is_importable: !unit.dotted.is_empty(),
        ..Default::default()
    };
    imports.module_path = unit.dotted.clone();
    imports.std_privilege = unit.dotted.starts_with("std.")
        || (unit.dotted.is_empty() && names_a_std_module(&unit.source.name));
    let direct: HashSet<usize> = unit
        .imports
        .iter()
        .map(crate::modules::ResolvedImport::target)
        .collect();
    for (index, module) in exports.iter().enumerate() {
        if index == unit_index {
            continue;
        }
        for (ty, entries) in &module.methods {
            if aelys_sema::is_builtin_key(ty) {
                if direct.contains(&index) {
                    imports
                        .methods
                        .entry(ty.clone())
                        .or_default()
                        .extend(entries.iter().cloned());
                }
                continue;
            }
            imports
                .methods
                .entry(ty.clone())
                .or_default()
                .extend(entries.iter().cloned());
        }
        for def in module.traits.values() {
            imports
                .all_traits
                .insert(def.qualified.clone(), def.clone());
        }
        imports.impls.extend(module.impls.iter().cloned());
    }
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
                imports.symbol_sources.push(exports[*target].clone());
                for (item, local) in names {
                    if let Err(kind) = import_symbol(&mut imports, &exports[*target], item, local)
                        && !tolerant
                    {
                        return Err(import_error(unit, *span, kind));
                    }
                }
            }
        }
    }
    Ok(imports)
}

fn import_symbol(
    imports: &mut aelys_sema::ModuleImports,
    module: &aelys_sema::ModuleExports,
    item: &str,
    local: &str,
) -> Result<(), CompileErrorKind> {
    use aelys_sema::Lookup;

    let not_public = || CompileErrorKind::SymbolNotPublic {
        symbol: item.to_string(),
        module: module.path.clone(),
    };
    if let Some((ty, variant)) = item.split_once("::") {
        return match module.module_type(ty) {
            Lookup::Found(found) => {
                imports
                    .variants
                    .insert(local.to_string(), (found.clone(), variant.to_string()));
                Ok(())
            }
            Lookup::NotPublic | Lookup::Missing => Err(CompileErrorKind::SymbolNotFound {
                symbol: item.replace("::", "."),
                module: module.path.clone(),
            }),
        };
    }
    match module.value(item) {
        Lookup::Found(found) => {
            imports.values.insert(local.to_string(), found.clone());
            return Ok(());
        }
        Lookup::NotPublic => return Err(not_public()),
        Lookup::Missing => {}
    }
    if let Some(def) = module.traits.get(item) {
        if !def.is_pub {
            return Err(not_public());
        }
        imports.traits.insert(local.to_string(), def.clone());
        return Ok(());
    }
    if let Some(def) = module.aliases.get(item) {
        if !def.is_pub {
            return Err(not_public());
        }
        imports.aliases.insert(local.to_string(), def.clone());
        return Ok(());
    }
    match module.module_type(item) {
        Lookup::Found(ty) => {
            imports.types.insert(local.to_string(), ty.clone());
            Ok(())
        }
        Lookup::NotPublic => Err(not_public()),
        Lookup::Missing => Err(CompileErrorKind::SymbolNotFound {
            symbol: item.to_string(),
            module: module.path.clone(),
        }),
    }
}

fn names_a_std_module(path: &str) -> bool {
    let path = Path::new(path);
    let (Some(stem), Some(dir)) = (path.file_stem(), path.parent()) else {
        return false;
    };
    dir.file_name().is_some_and(|d| d == "std")
        && dir.join("prelude.aelys").is_file()
        && crate::EMBEDDED_STD.iter().any(|(name, _)| stem == *name)
}

fn called_names(function: &aelys_air::AirFunction, out: &mut HashSet<String>) {
    use aelys_air::{AirStmtKind, AirTerminator, Callee, Rvalue};
    let mut take = |callee: &Callee| {
        if let Callee::Named(name) | Callee::Extern(name, _) = callee {
            out.insert(name.clone());
        }
    };
    for block in &function.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                AirStmtKind::CallVoid { func, .. } => take(func),
                AirStmtKind::Assign {
                    rvalue: Rvalue::Call { func, .. },
                    ..
                } => take(func),
                _ => {}
            }
        }
        if let AirTerminator::Invoke { func, .. } = &block.terminator {
            take(func);
        }
    }
}

// a prelude-only module is reached through its types alone, so only what another module calls survives
fn drop_unreached_prelude_functions(
    fronts: &[ModuleFront],
    programs: &mut [aelys_air::AirProgram],
) {
    if !fronts.iter().any(|front| front.functions_unreachable) {
        return;
    }
    let mut wanted: HashSet<String> = HashSet::new();
    for (front, program) in fronts.iter().zip(programs.iter()) {
        if !front.functions_unreachable {
            for function in &program.functions {
                called_names(function, &mut wanted);
            }
        }
    }
    for (front, program) in fronts.iter().zip(programs.iter_mut()) {
        if !front.functions_unreachable {
            continue;
        }
        loop {
            let before = wanted.len();
            for function in &program.functions {
                if wanted.contains(&function.name) {
                    called_names(function, &mut wanted);
                }
            }
            if wanted.len() == before {
                break;
            }
        }
        program
            .functions
            .retain(|function| wanted.contains(&function.name));
    }
}

// a prelude name loses, silently, to anything the unit binds itself in the same namespace
fn fall_back_to_prelude(
    imports: &mut aelys_sema::ModuleImports,
    scope: &aelys_sema::ModuleImports,
    bound_types: &HashSet<String>,
    bound_values: &HashSet<String>,
) {
    for (name, exports) in &scope.namespaces {
        if !bound_values.contains(name) {
            imports
                .namespaces
                .entry(name.clone())
                .or_insert_with(|| exports.clone());
        }
    }
    for (name, item) in &scope.values {
        if !bound_values.contains(name) {
            imports
                .values
                .entry(name.clone())
                .or_insert_with(|| item.clone());
        }
    }
    for (name, ty) in &scope.types {
        if !bound_types.contains(name) {
            imports
                .types
                .entry(name.clone())
                .or_insert_with(|| ty.clone());
        }
    }
}

fn widen_with_exports(scope: &mut aelys_sema::ModuleImports, exports: &aelys_sema::ModuleExports) {
    for (name, item) in &exports.values {
        if item.is_pub {
            scope.values.insert(name.clone(), item.clone());
        }
    }
    for (name, ty) in &exports.types {
        if ty.is_pub {
            scope.types.insert(name.clone(), ty.clone());
        }
    }
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

fn sourced(warnings: Vec<Warning>, src: &Arc<Source>) -> Vec<Warning> {
    warnings
        .into_iter()
        .map(|warning| {
            if warning.source.is_none() {
                warning.with_source(src.clone())
            } else {
                warning
            }
        })
        .collect()
}

struct Typed {
    inference: aelys_sema::InferenceResult,
    globals: HashSet<String>,
}

fn exports_of(
    unit: &crate::modules::ModuleUnit,
    program: &aelys_sema::TypedProgram,
) -> Result<aelys_sema::ModuleExports, AelysError> {
    aelys_sema::modules::collect_exports(&unit.dotted, program).map_err(|message| {
        backend_diagnostic_error(
            unit.source.clone(),
            fallback_source_span(unit.source.as_ref()),
            "exports",
            message,
            None,
            None,
            Fault::Compiler,
        )
    })
}

fn typecheck(
    unit: &crate::modules::ModuleUnit,
    imports: aelys_sema::ModuleImports,
) -> Result<Typed, AelysError> {
    let src = unit.source.clone();
    let globals = imported_global_symbols(&imports);
    let inference = crate::timing::phase("sema", || {
        aelys_sema::TypeInference::infer_program_full(
            unit.stmts.clone(),
            src.clone(),
            imports,
            known_globals(),
        )
    })
    .map_err(|errors| sema_errors_to_diagnostics(errors, src))?;
    Ok(Typed { inference, globals })
}

fn front_stage(
    unit: &crate::modules::ModuleUnit,
    typed: Typed,
    bir_imports: &aelys_air::bir::Imports,
) -> Result<(ModuleFront, aelys_air::bir::Checked), AelysError> {
    let src = unit.source.clone();
    let Typed { inference, globals } = typed;
    let aelys_sema::InferenceResult {
        program,
        warnings,
        deferred_errors,
        ..
    } = inference;
    let reserved = aelys_air::symbols::reserved_user_names(&program);
    let foreign_types = aelys_air::symbols::foreign_signature_violations(&program);
    let mut collected_errors = Vec::new();
    let mut effects = std::collections::HashMap::new();
    let mut chains = std::collections::HashMap::new();
    let mut externs = std::collections::HashMap::new();
    let checked = match crate::timing::phase("bir", || {
        aelys_air::bir::check_with_imports(program, bir_imports)
    }) {
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
            if !published.foreign_clashes.is_empty() {
                collected_errors.push(foreign_clash_errors_to_error(
                    published.foreign_clashes,
                    src.clone(),
                ));
            }
            externs = published.externs;
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
    if !foreign_types.is_empty() {
        collected_errors.push(foreign_signature_errors_to_error(foreign_types, src.clone()));
    }
    if !collected_errors.is_empty() {
        return Err(multiple_diagnostics(collected_errors));
    }
    let checked = match checked {
        Some(checked) => checked,
        None => return Err(multiple_diagnostics(collected_errors)),
    };

    let exports = Arc::new(exports_of(unit, checked.program())?);

    Ok((
        ModuleFront {
            dotted: unit.dotted.clone(),
            source: src.clone(),
            globals,
            exports,
            effects,
            chains,
            externs,
            warnings: sourced(warnings, &src),
            functions_unreachable: false,
        },
        checked,
    ))
}

struct StageFailure {
    error: AelysError,
}

impl From<AelysError> for StageFailure {
    fn from(error: AelysError) -> Self {
        StageFailure { error }
    }
}

// every check a -o level can change the input of is in here, so above -o0 this runs twice
fn air_stage(
    fronts: &[ModuleFront],
    typed: &[&aelys_sema::TypedProgram],
    root: &Arc<Source>,
    sources: &ModuleSources,
    opt_level: OptimizationLevel,
    judge_instances: bool,
    group_start: Option<usize>,
) -> Result<aelys_air::AirProgram, StageFailure> {
    if opt_level != OptimizationLevel::None && aelys_air::ablation::optimized_stage_refuses() {
        return Err(backend_diagnostic_error(
            root.clone(),
            fallback_source_span(root.as_ref()),
            "ablation",
            "the optimized stage is refused by the ablation switch",
            None,
            None,
            Fault::Compiler,
        )
        .into());
    }
    let mut programs: Vec<aelys_air::AirProgram> = Vec::with_capacity(fronts.len());
    let mut bir_imports = aelys_air::bir::Imports::default();
    let mut fresh_returns: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut retaining_fns: std::collections::HashSet<String> = std::collections::HashSet::new();

    let mut group = group_start.map(|start| group::GroupLowering::new(fronts, typed, start));
    let mut before_group = None;
    let mut file = 0;
    while file < fronts.len() {
        let (front, typed_program) = (&fronts[file], typed[file]);
        if let Some(lowering) = group.as_ref().filter(|g| g.start == file) {
            let snapshot = before_group.get_or_insert_with(|| {
                (
                    fresh_returns.clone(),
                    retaining_fns.clone(),
                    bir_imports.clone(),
                )
            });
            (fresh_returns, retaining_fns, bir_imports) = snapshot.clone();
            programs.truncate(file);
            retaining_fns.extend(lowering.assumed.iter().cloned());
            for later in &fronts[file..] {
                bir_imports.effects.extend(later.effects.clone());
                bir_imports.chains.extend(later.chains.clone());
            }
        }
        let grouped = group.as_ref().filter(|g| file >= g.start);
        let src = front.source.clone();
        let mut known_fresh = fresh_returns.clone();
        let mut structs: Vec<aelys_air::AirStructDef> =
            programs.iter().flat_map(|p| p.structs.clone()).collect();
        let mut enums: Vec<aelys_air::AirEnumDef> =
            programs.iter().flat_map(|p| p.enums.clone()).collect();
        if let Some(g) = grouped {
            known_fresh.extend(g.assumed_by(file));
            structs.extend(g.structs_after(file));
            enums.extend(g.enums_after(file));
        }
        let imported = aelys_air::lower::Imported {
            globals: front.globals.clone(),
            structs,
            enums,
            bir: bir_imports.clone(),
            fresh_returns: known_fresh.clone(),
            retaining_fns: retaining_fns.clone(),
            file: file as u32,
        };

        let mut air = crate::timing::phase("lower", || {
            aelys_air::lower::try_lower_with_imports(typed_program, imported)
        })
        .map_err(|failure| match failure {
            aelys_air::lower::LowerFailure::Borrow(diags) => {
                bir_diagnostics_to_error(diags, src.clone())
            }
            aelys_air::lower::LowerFailure::Lowering(errors) => {
                if errors.is_empty() {
                    backend_diagnostic_error(
                        src.clone(),
                        fallback_source_span(src.as_ref()),
                        "air-lowering",
                        "AIR lowering failed with an unknown error",
                        None,
                        None,
                        Fault::Compiler,
                    )
                } else {
                    lowering_errors_to_error(errors, src.clone())
                }
            }
        })?;

        // mono is function-destroying as well as function-creating: two block-nested `fn g<t>` in
        let duplicates = aelys_air::symbols::duplicate_symbols(&air);
        if !duplicates.is_empty() {
            return Err(duplicate_symbol_errors_to_error(
                duplicates,
                typed_program,
                &air,
                src.clone(),
            )
            .into());
        }

        aelys_air::modules::qualify(&mut air, &front.dotted);
        let carriers = aelys_air::counts::Carriers::merged(&air.structs, &air.enums);
        let proven =
            aelys_air::lower::fresh_returning_functions(&air.functions, &carriers, &known_fresh);
        if let Some(g) = group.as_mut().filter(|g| file >= g.start) {
            g.record_unproven(&air.functions, &carriers, &proven);
        }
        fresh_returns.extend(proven);
        retaining_fns.extend(aelys_air::lower::retaining_functions(&air.functions));

        let runtime_claims = aelys_air::symbols::reserved_runtime_symbols(&air, BOOTSTRAP_BUILTINS);
        if !runtime_claims.is_empty() {
            return Err(runtime_symbol_errors_to_error(runtime_claims, &air, src.clone()).into());
        }

        for (name, set) in &front.effects {
            bir_imports.effects.insert(name.clone(), *set);
        }
        for (name, steps) in &front.chains {
            bir_imports.chains.insert(name.clone(), steps.clone());
        }
        for (name, sig) in &front.externs {
            bir_imports.externs.insert(name.clone(), sig.clone());
        }

        programs.push(air);
        file += 1;
        if file == fronts.len()
            && let Some(g) = group.as_mut()
            && g.retract_unproven()
        {
            file = g.start;
        }
    }

    drop_unreached_prelude_functions(fronts, &mut programs);
    let std_functions: HashSet<String> = fronts
        .iter()
        .zip(&programs)
        .filter(|(front, _)| front.dotted.starts_with("std."))
        .flat_map(|(_, program)| program.functions.iter().map(|f| f.name.clone()))
        .collect();

    let typed_program = typed.last().copied().expect("a graph always holds its root");
    let src = root.clone();

    let air = aelys_air::modules::merge(programs);

    let duplicates = aelys_air::symbols::duplicate_symbols(&air);
    if !duplicates.is_empty() {
        return Err(
            duplicate_symbol_errors_to_error(duplicates, typed_program, &air, src.clone()).into(),
        );
    }

    let mut air =
        crate::timing::phase("mono", || aelys_air::mono::monomorphize(air)).map_err(|errors| {
            mono_errors_to_error(errors, fallback_source_span(src.as_ref()), src.clone())
        })?;
    aelys_air::passes::never_arms::cut_never_arms(&mut air);
    let effect_errors = {
        let mut effects = std::collections::HashMap::new();
        let mut declared = HashSet::new();
        for (front, typed_program) in fronts.iter().zip(typed.iter()) {
            for (name, set) in &front.effects {
                effects.insert(name.clone(), *set);
            }
            for stmt in &typed_program.stmts {
                if let aelys_sema::TypedStmtKind::Function(func) = &stmt.kind
                    && func.declared_nogc
                {
                    declared.insert(aelys_sema::modules::qualify_value(&front.dotted, &func.name));
                }
            }
        }
        aelys_air::passes::trait_effects::check_instance_effects(&mut air, &effects, &declared)
    };
    if !effect_errors.is_empty() {
        let files: Vec<Arc<Source>> = fronts.iter().map(|f| f.source.clone()).collect();
        return Err(
            instance_surface_errors_to_error(effect_errors, &air, &files, src.clone()).into(),
        );
    }
    aelys_air::passes::generic_print::format_strings_by_copy(&mut air);
    // mono joins the name and its type arguments with `_`, so `f<a_b>` and `f_a<b>` land on one
    let duplicates = aelys_air::symbols::duplicate_symbols(&air);
    if !duplicates.is_empty() {
        return Err(
            duplicate_symbol_errors_to_error(duplicates, typed_program, &air, src.clone()).into(),
        );
    }
    let instance_errors = if judge_instances {
        let mut errors = aelys_air::passes::generic_print::check_generic_print(&air);
        errors.extend(aelys_air::passes::instance_surface::check_instance_surface(
            &air,
        ));
        errors
    } else {
        Vec::new()
    };
    if !instance_errors.is_empty() {
        let files: Vec<Arc<Source>> = fronts.iter().map(|f| f.source.clone()).collect();
        return Err(
            instance_surface_errors_to_error(instance_errors, &air, &files, src.clone()).into(),
        );
    }
    if let Err(errors) = aelys_air::passes::vec_surface::check_vec_surface(&air) {
        return Err(vec_surface_errors_to_error(errors, &air, src.clone()).into());
    }
    let std_instances: HashSet<aelys_air::FunctionId> = air
        .mono_instances
        .iter()
        .filter(|m| std_functions.contains(&m.original_name))
        .map(|m| m.result)
        .collect();
    aelys_air::passes::unreached::drop_unreached(&mut air, |f| {
        std_functions.contains(&f.name) || std_instances.contains(&f.id)
    });
    let layout_errors = aelys_air::layout::compute_layouts(&mut air);
    if !layout_errors.is_empty() {
        let messages: Vec<String> = layout_errors.iter().map(|e| e.to_string()).collect();
        let owner = sources.owner(&joined_with_semicolons(&messages));
        let faulted: Vec<FaultMessage> = layout_errors
            .iter()
            .zip(messages)
            .map(|(error, message)| FaultMessage {
                fault: error.fault(),
                message,
            })
            .collect();
        return Err(join_by_fault(
            faulted,
            anchor_in(&air, owner.as_ref(), &src),
            owner.clone(),
            "air-layout",
            joined_with_semicolons,
        )
        .into());
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
            Fault::Compiler,
        )
    })?;
    aelys_air::passes::copy_elim::eliminate_copies(&mut air);
    aelys_air::passes::dead_locals::eliminate_dead_locals(&mut air);

    // only to keep the -o0 baseline byte-identical, never for soundness
    if opt_level >= OptimizationLevel::Basic {
        aelys_air::passes::rc_elision::eliminate_redundant_rc(&mut air);
    }

    // runs at every -o level: the leaks it closes are counted rows, not an optimisation
    aelys_air::passes::temp_elision::elide_temporaries(&mut air);

    if let Err(validation_errors) = aelys_air::passes::validate::validate_air(&air) {
        let messages: Vec<String> = validation_errors.iter().map(|e| e.to_string()).collect();
        let owner = sources.owner(&joined_with_newlines(&messages));
        return Err(join_by_fault(
            all_compiler(messages),
            anchor_in(&air, owner.as_ref(), &src),
            owner.clone(),
            "air-validation",
            joined_with_newlines,
        )
        .into());
    }

    let root = &fronts.last().expect("a graph always holds its root").dotted;
    if !root.is_empty() {
        aelys_air::modules::rename_function(&mut air, &format!("{root}.main"), "main");
    }
    Ok(air)
}

fn joined_with_semicolons(messages: &[String]) -> String {
    messages.join("; ")
}

fn joined_with_newlines(messages: &[String]) -> String {
    messages.join("\n")
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
    sources: &SourceOptions,
) -> Result<LoweringArtifacts, AelysError> {
    let mut graph = crate::modules::discover(path, sources).map_err(|failure| failure.error)?;
    crate::timing::phase("strip", || {
        crate::modules::strip_unnamed_std_functions(&mut graph.units)
    });

    let src = graph.root().source.clone();
    let module_sources = ModuleSources::new(
        &graph
            .units
            .iter()
            .map(|unit| (unit.dotted.clone(), unit.source.clone()))
            .collect::<Vec<_>>(),
        src.clone(),
    );

    let mut exports: Vec<Arc<aelys_sema::ModuleExports>> = Vec::new();
    let mut fronts: Vec<ModuleFront> = Vec::new();
    let mut checked: Vec<aelys_air::bir::Checked> = Vec::new();
    let mut bir_imports = aelys_air::bir::Imports::default();
    let mut prelude_scope: Option<aelys_sema::ModuleImports> = None;
    let group_start = graph
        .units
        .iter()
        .position(|unit| graph.grouped && !aelys_sema::modules::is_std_module(&unit.dotted));
    let mut grouped: Vec<Typed> = Vec::new();
    for (index, unit) in graph.units.iter().enumerate() {
        if Some(index) == group_start {
            group::declare(&graph.units, index, &mut exports, prelude_scope.as_ref())?;
        }
        let mut imports = build_imports(unit, index, &exports, false)?;
        let prelude_own_scope = (Some(index) == graph.prelude).then(|| imports.clone());
        if let Some(scope) = &prelude_scope {
            fall_back_to_prelude(&mut imports, scope, &unit.bound_types, &unit.bound_values);
        }
        if group_start.is_some_and(|start| index >= start) {
            aelys_sema::localize::localize_imports(&mut imports);
            let typed = typecheck(unit, imports)?;
            exports[index] = Arc::new(exports_of(unit, &typed.inference.program)?);
            grouped.push(typed);
            continue;
        }
        let (front, unit_checked) = crate::timing::phase("front", || {
            front_stage(unit, typecheck(unit, imports)?, &bir_imports)
        })?;
        if let Some(mut scope) = prelude_own_scope {
            widen_with_exports(&mut scope, &front.exports);
            prelude_scope = Some(scope);
        }
        for (name, set) in &front.effects {
            bir_imports.effects.insert(name.clone(), *set);
        }
        import_chains_and_externs(&mut bir_imports, &front);
        exports.push(front.exports.clone());
        fronts.push(front);
        checked.push(unit_checked);
    }
    if let Some(start) = group_start {
        let units = &graph.units[start..];
        group::settle_effects(units, &grouped, &mut bir_imports);
        for (unit, typed) in units.iter().zip(grouped) {
            let (front, unit_checked) = front_stage(unit, typed, &bir_imports)?;
            import_chains_and_externs(&mut bir_imports, &front);
            fronts.push(front);
            checked.push(unit_checked);
        }
    }
    if prelude_scope
        .as_ref()
        .is_some_and(|scope| scope.values.is_empty() && scope.namespaces.is_empty())
    {
        for (front, unit) in fronts.iter_mut().zip(&graph.units) {
            front.functions_unreachable = unit.prelude_only;
        }
    }

    let reference = (opt_level != OptimizationLevel::None).then(|| {
        crate::timing::phase("air (reference)", || {
            let unoptimized: Vec<&aelys_sema::TypedProgram> =
                checked.iter().map(|unit| unit.program()).collect();
            air_stage(
                &fronts,
                &unoptimized,
                &src,
                &module_sources,
                OptimizationLevel::None,
                true,
                group_start,
            )
        })
    });
    let reference = reference.transpose().map_err(|failure| failure.error)?;

    let mut warnings: Vec<Warning> = Vec::new();
    let mut optimized: Vec<aelys_sema::TypedProgram> = Vec::with_capacity(checked.len());
    for (front, unit_checked) in fronts.iter_mut().zip(checked) {
        let mut optimizer = Optimizer::new(opt_level);
        let program = crate::timing::phase("opt", || optimizer.optimize(unit_checked));
        warnings.append(&mut front.warnings);
        warnings.extend(sourced(optimizer.take_warnings(), &front.source));
        optimized.push(program);
    }

    let optimized_refs: Vec<&aelys_sema::TypedProgram> = optimized.iter().collect();
    // the instance verdict reads the declared types, which optimization may drop
    let actual = crate::timing::phase("air", || {
        air_stage(
            &fronts,
            &optimized_refs,
            &src,
            &module_sources,
            opt_level,
            opt_level == OptimizationLevel::None,
            group_start,
        )
    });

    let level = format!("-O{}", opt_level.numeric());
    let mut required_externs: Option<Vec<String>> = None;
    let air = match (reference, actual) {
        (None, actual) => actual.map_err(|failure| failure.error)?,
        (Some(unoptimized), Ok(air)) => {
            required_externs = Some(aelys_air::symbols::referenced_extern_symbols(&unoptimized));
            air
        }
        (Some(_), Err(optimized_error)) => {
            return Err(optimization_verdict_split(
                src.clone(),
                level,
                &optimized_error.error,
                UNOPTIMIZED_LEVEL.to_string(),
            ));
        }
    };

    let required_externs = required_externs
        .unwrap_or_else(|| aelys_air::symbols::referenced_extern_symbols(&air));
    let files = graph
        .units
        .iter()
        .filter_map(|unit| Some((unit.file.clone()?, unit.source.clone())))
        .collect();
    Ok(LoweringArtifacts {
        air,
        source: src,
        warnings,
        required_externs,
        files,
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
    let require = aelys_air::symbols::referenced_extern_symbols(air);
    compile_air_with_llvm(path, air, opt_level, false, runtime, src, &require)
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
    let artifacts = lower_file_to_air_with_source(path, opt_level, &SourceOptions::default())?;
    compile_air_with_llvm(
        path,
        &artifacts.air,
        opt_level,
        emit_llvm_ir,
        runtime,
        artifacts.source,
        &artifacts.required_externs,
    )
}

pub fn compile_file_with_llvm_with_warnings(
    path: &Path,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
    runtime: RuntimeVariant,
) -> Result<Vec<Warning>, AelysError> {
    compile_file_with_llvm_linked(
        path,
        opt_level,
        emit_llvm_ir,
        runtime,
        &LinkRequirement::default(),
    )
}

pub fn compile_file_with_llvm_linked(
    path: &Path,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
    runtime: RuntimeVariant,
    link: &LinkRequirement,
) -> Result<Vec<Warning>, AelysError> {
    compile_file_with_llvm_sources(
        path,
        opt_level,
        emit_llvm_ir,
        runtime,
        link,
        &SourceOptions::default(),
        None,
    )
}

pub fn compile_file_with_llvm_sources(
    path: &Path,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
    runtime: RuntimeVariant,
    link: &LinkRequirement,
    sources: &SourceOptions,
    output: Option<&Path>,
) -> Result<Vec<Warning>, AelysError> {
    compile_file_with_llvm_artifact(
        path,
        opt_level,
        emit_llvm_ir,
        runtime,
        link,
        sources,
        output,
    )
    .map(|(warnings, _)| warnings)
}

pub fn compile_file_with_llvm_artifact(
    path: &Path,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
    runtime: RuntimeVariant,
    link: &LinkRequirement,
    sources: &SourceOptions,
    output: Option<&Path>,
) -> Result<(Vec<Warning>, CompiledArtifact), AelysError> {
    crate::timing::phase("total", || {
        let artifacts = lower_file_to_air_with_source(path, opt_level, sources)?;
        let artifact = compile_air_with_llvm_linked(
            path,
            &artifacts.air,
            opt_level,
            emit_llvm_ir,
            runtime,
            artifacts.source.clone(),
            link,
            output,
            ObjectPath::Private,
            None,
            &artifacts.required_externs,
        )?;
        Ok((artifacts.warnings, artifact))
    })
}

pub struct BuildRequest<'a> {
    pub root: &'a Path,
    pub opt_level: OptimizationLevel,
    pub runtime: RuntimeVariant,
    pub link: &'a LinkRequirement,
    pub core_lib: &'a Path,
    pub output: &'a Path,
    pub object: &'a Path,
}

pub struct Lowered {
    pub warnings: Vec<Warning>,
    pub files: Vec<(PathBuf, Arc<Source>)>,
}

pub fn build(request: &BuildRequest<'_>) -> Result<(Lowered, CompiledArtifact), AelysError> {
    let artifacts =
        lower_file_to_air_with_source(request.root, request.opt_level, &SourceOptions::default())?;
    let artifact = compile_air_with_llvm_linked(
        request.root,
        &artifacts.air,
        request.opt_level,
        false,
        request.runtime,
        artifacts.source.clone(),
        request.link,
        Some(request.output),
        ObjectPath::At(request.object),
        Some(request.core_lib),
        &artifacts.required_externs,
    )?;
    let lowered = Lowered {
        warnings: artifacts.warnings,
        files: artifacts.files,
    };
    Ok((lowered, artifact))
}

pub fn check_file(path: &Path) -> Result<Lowered, AelysError> {
    lower_file_to_air_with_source(path, OptimizationLevel::None, &SourceOptions::default()).map(
        |artifacts| Lowered {
            warnings: artifacts.warnings,
            files: artifacts.files,
        },
    )
}

fn run_process_c_locale(program: &str, args: &[String]) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args);
    command.env("LC_ALL", "C");
    finish_process(program, command)
}

fn capture_process(program: &str, args: &[String]) -> Option<String> {
    let output = Command::new(program)
        .args(args)
        .env("LC_ALL", "C")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

fn run_process_in_dir(program: &str, args: &[String], dir: Option<&Path>) -> Result<(), String> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(path) = dir {
        command.current_dir(path);
    }
    finish_process(program, command)
}

fn finish_process(program: &str, mut command: Command) -> Result<(), String> {
    let output = command
        .output()
        .map_err(|err| format!("failed to run `{}`: {}", program, err))?;

    if output.status.success() {
        return Ok(());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let mut message = match output.status.code() {
        Some(code) => format!("`{program}` exited with status {code}"),
        None => format!("`{program}` was stopped by a signal"),
    };
    for (stream, text) in [("stdout", stdout.trim()), ("stderr", stderr.trim())] {
        if !text.is_empty() {
            message.push_str(&format!("\n{stream}:\n{text}"));
        }
    }
    Err(message)
}

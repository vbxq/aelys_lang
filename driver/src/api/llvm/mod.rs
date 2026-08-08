mod core_lib;
mod diagnostics;
mod link;
mod lower;
mod runtime;

pub use runtime::RuntimeVariant;

use aelys_common::Warning;
use aelys_common::error::AelysError;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::{OptimizationLevel, Optimizer};
use aelys_syntax::Source;
use std::collections::HashSet;
use std::path::Path;
use std::process::Command;
use std::sync::Arc;

use diagnostics::{
    backend_diagnostic_error, bir_diagnostics_to_error, fallback_source_span,
    load_source_for_diagnostics, mono_errors_to_error, program_anchor_span,
    sema_errors_to_diagnostics, vec_surface_errors_to_error,
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
        HashSet::new(),
        known_globals,
    )
    .map_err(|errors| sema_errors_to_diagnostics(errors, src))?;

    Ok(inference.program)
}

pub fn lower_file_to_air(
    path: &Path,
    opt_level: OptimizationLevel,
) -> Result<aelys_air::AirProgram, String> {
    let artifacts =
        lower_file_to_air_with_source(path, opt_level).map_err(|err| err.to_string())?;
    Ok(artifacts.air)
}

fn lower_file_to_air_with_source(
    path: &Path,
    opt_level: OptimizationLevel,
) -> Result<LoweringArtifacts, AelysError> {
    let content = std::fs::read_to_string(path).map_err(|err| {
        let source = load_source_for_diagnostics(path);
        backend_diagnostic_error(
            source.clone(),
            fallback_source_span(source.as_ref()),
            "driver",
            format!("failed to read {}: {}", path.display(), err),
            None,
            None,
        )
    })?;

    let name = path.display().to_string();
    let src = Source::new(&name, &content);

    let tokens = Lexer::with_source(src.clone()).scan()?;
    let stmts = Parser::new(tokens, src.clone()).parse()?;

    let known_globals: HashSet<String> = BOOTSTRAP_BUILTINS.iter().map(|s| s.to_string()).collect();

    let inference = aelys_sema::TypeInference::infer_program_full(
        stmts,
        src.clone(),
        HashSet::new(),
        known_globals,
    )
    .map_err(|errors| sema_errors_to_diagnostics(errors, src.clone()))?;

    let checked = aelys_air::bir::check(inference.program)
        .map_err(|errors| bir_diagnostics_to_error(errors, src.clone()))?;

    let mut optimizer = Optimizer::new(opt_level);
    let typed_program = optimizer.optimize(checked);

    let air = aelys_air::lower::try_lower(&typed_program).map_err(|failure| match failure {
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
    })?;
    let mut air = aelys_air::mono::monomorphize(air).map_err(|errors| {
        mono_errors_to_error(errors, fallback_source_span(src.as_ref()), src.clone())
    })?;
    // program: before this point a generic body still hides its instantiations
    if let Err(errors) = aelys_air::passes::vec_surface::check_vec_surface(&air) {
        return Err(vec_surface_errors_to_error(errors, &air, src.clone()));
    }
    let layout_errors = aelys_air::layout::compute_layouts(&mut air);
    if !layout_errors.is_empty() {
        return Err(backend_diagnostic_error(
            src.clone(),
            program_anchor_span(&air, src.as_ref()),
            "air-layout",
            layout_errors.join("; "),
            None,
            None,
        ));
    }

    // must run after compute_layouts, it reads the filled field offsets
    air.rc_type_table = aelys_air::rc_types::collect_rc_types(&air).map_err(|e| {
        backend_diagnostic_error(
            src.clone(),
            program_anchor_span(&air, src.as_ref()),
            "air-rc-types",
            e.to_string(),
            None,
            None,
        )
    })?;
    aelys_air::passes::copy_elim::eliminate_copies(&mut air);
    aelys_air::passes::dead_locals::eliminate_dead_locals(&mut air);

    // must run after copy_elim/dead_locals to see the final call sites; gated at >= Basic
    // only to keep the -O0 baseline byte-identical, never for soundness
    if opt_level >= OptimizationLevel::Basic {
        aelys_air::passes::rc_elision::eliminate_redundant_rc(&mut air);
    }

    if let Err(validation_errors) = aelys_air::passes::validate::validate_air(&air) {
        let message = validation_errors
            .iter()
            .map(|e| e.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        return Err(backend_diagnostic_error(
            src.clone(),
            program_anchor_span(&air, src.as_ref()),
            "air-validation",
            message,
            None,
            None,
        ));
    }

    let warnings = inference
        .warnings
        .into_iter()
        .map(|warning| {
            if warning.source.is_none() {
                warning.with_source(src.clone())
            } else {
                warning
            }
        })
        .collect();

    Ok(LoweringArtifacts {
        air,
        source: src,
        warnings,
    })
}

/// proves a claim about compilation, not about a value. this is the only oracle in the tree
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
    // this 3-arg entry point links the default runtime variant, which is real refcounting
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

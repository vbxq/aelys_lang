use aelys_common::error::{AelysError, Fault};
use aelys_opt::OptimizationLevel;
use aelys_syntax::Source;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::core_lib::resolve_aelys_core_lib;
use super::diagnostics::{
    backend_diagnostic_error, llvm_backend_error_to_diagnostic, program_anchor_span,
};
use super::link::{LinkRequirement, link_native_executable, linked_library_claims_runtime_symbol};
use super::runtime::RuntimeVariant;

pub(super) fn compile_air_with_llvm(
    path: &Path,
    air: &aelys_air::AirProgram,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
    runtime: RuntimeVariant,
    source: Arc<Source>,
) -> Result<(), AelysError> {
    compile_air_with_llvm_linked(
        path,
        air,
        opt_level,
        emit_llvm_ir,
        runtime,
        source,
        &LinkRequirement::default(),
    )
}

pub(super) fn compile_air_with_llvm_linked(
    path: &Path,
    air: &aelys_air::AirProgram,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
    runtime: RuntimeVariant,
    source: Arc<Source>,
    link: &LinkRequirement,
) -> Result<(), AelysError> {
    let module_name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("aelys_module");
    let mut codegen = aelys_codegen::CodegenContext::new(module_name);

    codegen
        .compile(air)
        .map_err(|err| llvm_backend_error_to_diagnostic(err, air, source.clone()))?;
    codegen
        .optimize(opt_level.llvm_pass_pipeline(), opt_level.numeric())
        .map_err(|err| llvm_backend_error_to_diagnostic(err, air, source.clone()))?;

    if emit_llvm_ir {
        let mut ir_path = PathBuf::from(path);
        ir_path.set_extension("ll");
        let ir_path_str = ir_path.to_string_lossy().to_string();
        codegen
            .emit_ir(&ir_path_str)
            .map_err(|err| llvm_backend_error_to_diagnostic(err, air, source.clone()))?;
        return Ok(());
    }

    let object_path = object_path_for(path);
    let object_path_str = object_path.to_string_lossy().to_string();
    codegen
        .emit_object(&object_path_str, opt_level.numeric())
        .map_err(|err| llvm_backend_error_to_diagnostic(err, air, source.clone()))?;

    let has_main_entry = air
        .functions
        .iter()
        .any(|function| !function.is_extern && function.name == "main");
    if has_main_entry {
        let anchor = program_anchor_span(air, source.as_ref());
        let core_lib = resolve_aelys_core_lib(runtime).map_err(|message| {
            backend_diagnostic_error(
                source.clone(),
                anchor,
                "llvm-linker",
                message,
                None,
                None,
                Fault::Compiler,
            )
        })?;
        let exe_path = executable_path_for(path);
        link_native_executable(&object_path, &exe_path, &core_lib, runtime, link).map_err(
            |message| {
                backend_diagnostic_error(
                    source.clone(),
                    anchor,
                    "llvm-linker",
                    message,
                    None,
                    foreign_declaration_help(air, source.as_ref()),
                    Fault::Compiler,
                )
            },
        )?;
        if let Some(symbol) = linked_library_claims_runtime_symbol(&exe_path, &core_lib, link) {
            // the executable is the evidence, so it must not survive the verdict
            let _ = std::fs::remove_file(&exe_path);
            return Err(claimed_runtime_symbol_error(
                source.clone(),
                anchor,
                symbol,
                link,
            ));
        }
    }

    Ok(())
}

fn claimed_runtime_symbol_error(
    source: Arc<Source>,
    span: aelys_syntax::Span,
    symbol: String,
    link: &LinkRequirement,
) -> AelysError {
    AelysError::Compile(aelys_common::error::CompileError::new(
        aelys_common::error::CompileErrorKind::LinkedLibraryClaimsRuntimeSymbol {
            symbol,
            libraries: link.libraries.clone(),
        },
        span,
        source,
    ))
}

fn foreign_declaration_help(air: &aelys_air::AirProgram, source: &Source) -> Option<String> {
    let mut sites = Vec::new();
    for function in air.functions.iter().filter(|function| function.is_extern) {
        let line = function
            .span
            .map(|span| source.line_col_at_offset(span.lo as usize).0)
            .unwrap_or(0);
        sites.push(format!("`{}` at line {}", function.name, line));
    }
    if sites.is_empty() {
        return None;
    }
    Some(format!(
        "an external function is resolved at link time; add `-L <dir> -l <name>` for the library \
         that defines it. this program declares {}",
        sites.join(", ")
    ))
}

pub fn object_path_for(path: &Path) -> PathBuf {
    let mut object = path.to_path_buf();
    object.set_extension(if cfg!(windows) { "obj" } else { "o" });
    object
}

pub fn executable_path_for(path: &Path) -> PathBuf {
    let mut output = path.with_extension("");
    if cfg!(windows) {
        output.set_extension("exe");
    }
    output
}

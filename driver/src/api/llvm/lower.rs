use aelys_common::error::AelysError;
use aelys_opt::OptimizationLevel;
use aelys_syntax::Source;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use super::core_lib::resolve_aelys_core_lib;
use super::diagnostics::{
    backend_diagnostic_error, llvm_backend_error_to_diagnostic, program_anchor_span,
};
use super::link::link_native_executable;
use super::runtime::RuntimeVariant;

pub(super) fn compile_air_with_llvm(
    path: &Path,
    air: &aelys_air::AirProgram,
    opt_level: OptimizationLevel,
    emit_llvm_ir: bool,
    runtime: RuntimeVariant,
    source: Arc<Source>,
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
            backend_diagnostic_error(source.clone(), anchor, "llvm-linker", message, None, None)
        })?;
        let exe_path = executable_path_for(path);
        link_native_executable(&object_path, &exe_path, &core_lib, runtime).map_err(|message| {
            backend_diagnostic_error(source.clone(), anchor, "llvm-linker", message, None, None)
        })?;
    }

    Ok(())
}

fn object_path_for(path: &Path) -> PathBuf {
    let mut object = path.to_path_buf();
    object.set_extension(if cfg!(windows) { "obj" } else { "o" });
    object
}

fn executable_path_for(path: &Path) -> PathBuf {
    let mut output = path.with_extension("");
    if cfg!(windows) {
        output.set_extension("exe");
    }
    output
}

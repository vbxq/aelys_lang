// LLVM native compiler

use aelys_common::{WarningConfig, format_warnings};
use aelys_driver::{compile_file_with_llvm_with_warnings, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::path::{Path, PathBuf};

pub fn run_with_options(
    path: &str,
    output: Option<String>,
    opt_level: OptimizationLevel,
    _warn_config: WarningConfig,
    emit_air: bool,
    emit_llvm_ir: bool,
) -> Result<i32, String> {
    if emit_air {
        return emit_air_program(path, opt_level);
    }

    if output.is_some() {
        return Err("--output is not supported yet".to_string());
    }

    let warnings = compile_file_with_llvm_with_warnings(Path::new(path), opt_level, emit_llvm_ir)
        .map_err(|err| err.to_string())?;
    let filtered: Vec<_> = warnings
        .into_iter()
        .filter(|warning| _warn_config.is_enabled(&warning.kind))
        .collect();

    if !filtered.is_empty() {
        eprintln!("{}", format_warnings(&filtered));
    }
    if _warn_config.treat_as_error && !filtered.is_empty() {
        return Err(format!(
            "aborting due to {} warning(s) treated as errors",
            filtered.len()
        ));
    }

    if emit_llvm_ir {
        let mut ir_path = PathBuf::from(path);
        ir_path.set_extension("ll");
        eprintln!("Wrote {}", ir_path.display());
    }

    Ok(0)
}

pub fn emit_air_program(path: &str, opt_level: OptimizationLevel) -> Result<i32, String> {
    let path = Path::new(path);
    let air = lower_file_to_air(path, opt_level)?;
    print!("{}", aelys_air::print::print_program(&air));
    Ok(0)
}

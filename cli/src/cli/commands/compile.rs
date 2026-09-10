
use aelys_common::{ColorConfig, WarningConfig, format_warnings, render_summary};
use aelys_driver::{
    LinkRequirement, RuntimeVariant, SourceOptions, compile_file_with_llvm_sources,
    executable_path_for, lower_file_to_air_with_sources, object_path_for,
};
use aelys_opt::OptimizationLevel;
use std::path::{Path, PathBuf};

pub fn run_with_options(
    path: &str,
    output: Option<String>,
    opt_level: OptimizationLevel,
    runtime: RuntimeVariant,
    _warn_config: WarningConfig,
    emit_air: bool,
    emit_llvm_ir: bool,
    color: &ColorConfig,
    link: &LinkRequirement,
    sources: &SourceOptions,
) -> Result<i32, String> {
    if emit_air {
        return emit_air_program(path, opt_level, sources);
    }

    if output.is_some() {
        return Err("--output is not supported yet".to_string());
    }

    match compile_file_with_llvm_sources(
        Path::new(path),
        opt_level,
        emit_llvm_ir,
        runtime,
        link,
        sources,
    ) {
        Ok(warnings) => {
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
            } else if !executable_path_for(Path::new(path)).is_file() {
                eprintln!(
                    "Wrote {} (no `main`, so nothing was linked)",
                    object_path_for(Path::new(path)).display()
                );
            }

            Ok(0)
        }
        Err(err) => {
            let diagnostics = err.to_diagnostics();
            for diag in &diagnostics {
                eprint!("{}", diag.render(color));
            }
            let summary = render_summary(&diagnostics);
            if !summary.is_empty() {
                eprint!("{}", summary);
            }

            Err(String::new()) // Signal error exit without double printing
        }
    }
}

pub fn emit_air_program(
    path: &str,
    opt_level: OptimizationLevel,
    sources: &SourceOptions,
) -> Result<i32, String> {
    let path = Path::new(path);
    let air = lower_file_to_air_with_sources(path, opt_level, sources)?;
    print!("{}", aelys_air::print::print_program(&air));
    Ok(0)
}

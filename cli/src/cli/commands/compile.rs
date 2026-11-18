
use aelys_common::{ColorConfig, WarningConfig, format_warnings, render_summary};
use aelys_driver::{
    CompiledArtifact, LinkRequirement, RuntimeVariant, SourceOptions,
    compile_file_with_llvm_artifact, lower_file_to_air_with_sources,
};
use aelys_opt::OptimizationLevel;
use std::path::Path;

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

    let requested = output.as_deref().map(Path::new);

    match compile_file_with_llvm_artifact(
        Path::new(path),
        opt_level,
        emit_llvm_ir,
        runtime,
        link,
        sources,
        requested,
    ) {
        Ok((warnings, artifact)) => {
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

            match artifact {
                CompiledArtifact::LlvmIr(written) => eprintln!("Wrote {}", written.display()),
                CompiledArtifact::Executable(written) if requested.is_some() => {
                    eprintln!("Wrote {}", written.display())
                }
                CompiledArtifact::Executable(_) => {}
                CompiledArtifact::Object(written) => eprintln!(
                    "Wrote {} (an object file: no `main`, so nothing was linked)",
                    written.display()
                ),
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

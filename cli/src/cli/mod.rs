pub mod args;

pub mod commands {
    pub mod compile;
    pub mod explain;
}

use aelys_common::WarningConfig;

pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let invoked = std::env::args_os().next().and_then(|a| {
        std::path::Path::new(&a)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
    });
    aelys_common::diagnostic::render::set_program_name(invoked.as_deref().unwrap_or("aelys-cli"));
    let parsed = match args::parse_args(&args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("Error: {}", err);
            eprintln!("{}", args::usage());
            return 1;
        }
    };

    if matches!(parsed.command, args::Command::Help) {
        println!("{}", args::usage());
        return 0;
    }
    if matches!(parsed.command, args::Command::Version) {
        println!("{}", version_line());
        return 0;
    }

    dispatch(parsed).unwrap_or_else(|err| {
        eprintln!("{}", err);
        1
    })
}

pub fn version_line() -> String {
    format!(
        "Aelys v{} (embedded std {})",
        env!("CARGO_PKG_VERSION"),
        aelys_driver::embedded_std_hash()
    )
}

#[allow(dead_code)]
pub fn run_with_args(args: &[String]) -> Result<i32, String> {
    let parsed = args::parse_args(args)?;
    dispatch(parsed)
}

fn parse_warning_config(flags: &[String]) -> Result<WarningConfig, String> {
    let mut config = WarningConfig::new();
    for flag in flags {
        config.parse_flag(flag)?;
    }
    Ok(config)
}

fn color_config_from_choice(choice: &args::ColorChoice) -> aelys_common::ColorConfig {
    match choice {
        args::ColorChoice::Auto => aelys_common::ColorConfig::auto(),
        args::ColorChoice::Always => aelys_common::ColorConfig::always(),
        args::ColorChoice::Never => aelys_common::ColorConfig::never(),
    }
}

fn dispatch(parsed: args::ParsedArgs) -> Result<i32, String> {
    let warn_config = parse_warning_config(&parsed.warning_flags)?;
    let color = color_config_from_choice(&parsed.color);
    let runtime = parsed.runtime;
    let link = parsed.link;
    let sources = parsed.sources;

    match parsed.command {
        args::Command::Help => Ok(0),
        args::Command::Version => Ok(0),

        args::Command::Explain { code } => {
            commands::explain::run_explain(&code);
            Ok(0)
        }

        args::Command::Compile {
            path,
            output,
            emit_air,
            emit_llvm_ir,
        } => commands::compile::run_with_options(
            &path,
            output,
            parsed.opt_level,
            runtime,
            warn_config,
            emit_air,
            emit_llvm_ir,
            &color,
            &link,
            &sources,
        ),
    }
}

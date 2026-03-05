pub mod args;

pub mod commands {
    pub mod compile;
    pub mod explain;
}

use aelys_common::WarningConfig;

pub fn run() -> i32 {
    let args: Vec<String> = std::env::args().collect();
    let parsed = match args::parse_args(&args) {
        Ok(parsed) => parsed,
        Err(err) => {
            eprintln!("Error: {}", err);
            eprintln!("{}", args::usage());
            return 1;
        }
    };

    if matches!(parsed.command, args::Command::Help) {
        eprintln!("{}", args::usage());
        return 0;
    }
    if matches!(parsed.command, args::Command::Version) {
        eprintln!("Aelys v{}", env!("CARGO_PKG_VERSION"));
        return 0;
    }

    dispatch(parsed).unwrap_or_else(|err| {
        eprintln!("{}", err);
        1
    })
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
            warn_config,
            emit_air,
            emit_llvm_ir,
            &color,
        ),
    }
}

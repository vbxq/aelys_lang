pub mod args;
pub mod vm_config;

pub mod commands {
    pub mod asm;
    pub mod compile;
    pub mod repl;
    pub mod run;
}

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

    match dispatch(parsed) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("Error: {}", err);
            1
        }
    }
}

#[allow(dead_code)]
pub fn run_with_args(args: &[String]) -> Result<i32, String> {
    let parsed = args::parse_args(args)?;
    dispatch(parsed)
}

fn dispatch(parsed: args::ParsedArgs) -> Result<i32, String> {
    match parsed.command {
        args::Command::Help => Ok(0),
        args::Command::Run { path, program_args } => {
            commands::run::run_with_options(&path, program_args, parsed.vm_args, parsed.opt_level)
        }
        args::Command::Compile { path, output } => {
            if !parsed.vm_args.is_empty() {
                return Err("vm flags are only supported for run or repl".to_string());
            }
            commands::compile::run_with_options(&path, output, parsed.opt_level)
        }
        args::Command::Asm {
            path,
            output,
            stdout,
        } => {
            if !parsed.vm_args.is_empty() {
                return Err("vm flags are only supported for run or repl".to_string());
            }
            commands::asm::run_with_options(&path, output, stdout, parsed.opt_level)
        }
        args::Command::Repl => commands::repl::run_with_options(parsed.opt_level, parsed.vm_args),
        args::Command::Version => Ok(0),
    }
}

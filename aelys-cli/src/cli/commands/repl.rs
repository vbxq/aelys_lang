// TODO: rustyline for history/completion would be nice

use crate::cli::vm_config::parse_vm_args_or_error;
use aelys::{new_vm_with_config, run_with_vm_and_opt};
use aelys_opt::OptimizationLevel;
use std::io::{self, BufRead, IsTerminal, Write};

#[allow(dead_code)]
pub fn run_repl_with_io<R: BufRead, W: Write>(
    input: R,
    output: W,
    opt_level: OptimizationLevel,
    vm_args: Vec<String>,
) -> Result<(), String> {
    run_repl_core(input, output, opt_level, vm_args, false)
}

pub fn run_with_options(opt_level: OptimizationLevel, vm_args: Vec<String>) -> Result<i32, String> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let interactive = stdin.is_terminal() && stdout.is_terminal();
    run_repl_core(stdin.lock(), stdout.lock(), opt_level, vm_args, interactive)?;
    Ok(0)
}

fn run_repl_core<R: BufRead, W: Write>(
    mut input: R,
    mut output: W,
    opt_level: OptimizationLevel,
    vm_args: Vec<String>,
    interactive: bool,
) -> Result<(), String> {
    let parsed = parse_vm_args_or_error(&vm_args)?;
    let mut vm = new_vm_with_config(parsed.config, Vec::new()).map_err(|err| err.to_string())?;

    if interactive {
        writeln!(output, "Aelys REPL (type 'exit' to quit)").map_err(|err| err.to_string())?;
    }

    let mut line = String::new();
    loop {
        if interactive {
            write!(output, "aelys> ").map_err(|err| err.to_string())?;
            output.flush().map_err(|err| err.to_string())?;
        }

        line.clear();
        let bytes = input.read_line(&mut line).map_err(|err| err.to_string())?;
        if bytes == 0 {
            break;
        }

        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed == "exit" || trimmed == "quit" {
            break;
        }

        match run_with_vm_and_opt(&mut vm, trimmed, "<repl>", opt_level) {
            Ok(value) => {
                if !value.is_null() {
                    writeln!(output, "{}", value.to_string()).map_err(|err| err.to_string())?;
                }
            }
            Err(err) => {
                writeln!(output, "{}", err.to_string()).map_err(|err| err.to_string())?;
            }
        }
    }

    Ok(())
}

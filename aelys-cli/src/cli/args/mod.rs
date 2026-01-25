mod parse;
mod usage;

use aelys_opt::OptimizationLevel;

pub use parse::parse_args;
pub use usage::usage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Run {
        path: String,
        program_args: Vec<String>,
    },
    Compile {
        path: String,
        output: Option<String>,
    },
    Asm {
        path: String,
        output: Option<String>,
        stdout: bool,
    },
    Repl,
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArgs {
    pub command: Command,
    pub vm_args: Vec<String>,
    pub opt_level: OptimizationLevel,
}

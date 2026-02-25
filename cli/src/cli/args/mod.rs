mod parse;
mod usage;

use aelys_opt::OptimizationLevel;

pub use parse::parse_args;
pub use usage::usage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Compile {
        path: String,
        output: Option<String>,
        emit_air: bool,
        emit_llvm_ir: bool,
    },
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArgs {
    pub command: Command,
    pub opt_level: OptimizationLevel,
    pub warning_flags: Vec<String>,
}

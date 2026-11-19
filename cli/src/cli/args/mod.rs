mod parse;
mod usage;

use aelys_driver::{LinkRequirement, RuntimeVariant, SourceOptions};
use aelys_opt::OptimizationLevel;

pub use parse::parse_args;
pub use usage::usage;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColorChoice {
    Auto,
    Always,
    Never,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Compile {
        path: String,
        output: Option<String>,
        emit_air: bool,
        emit_llvm_ir: bool,
    },
    Explain {
        code: String,
    },
    Version,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedArgs {
    pub command: Command,
    pub opt_level: OptimizationLevel,
    pub runtime: RuntimeVariant,
    pub warning_flags: Vec<String>,
    pub color: ColorChoice,
    pub link: LinkRequirement,
    pub sources: SourceOptions,
}

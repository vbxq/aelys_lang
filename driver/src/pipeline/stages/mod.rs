mod air_lower;
mod compiler;
mod debug_strip;
mod lexer;
mod optimization;
mod parser;
mod type_inference;
mod vm;

pub use air_lower::AirLowerStage;
pub use compiler::CompilerStage;
pub use debug_strip::DebugStripStage;
pub use lexer::LexerStage;
pub use optimization::OptimizationStage;
pub use parser::ParserStage;
pub use type_inference::TypeInferenceStage;
pub use vm::VMStage;

mod compiler;
mod lexer;
mod optimization;
mod parser;
mod type_inference;
mod vm;

pub use compiler::CompilerStage;
pub use lexer::LexerStage;
pub use optimization::OptimizationStage;
pub use parser::ParserStage;
pub use type_inference::TypeInferenceStage;
pub use vm::VMStage;

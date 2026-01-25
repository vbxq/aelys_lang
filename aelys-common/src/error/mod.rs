use std::fmt;

pub mod compile;
pub mod runtime;
pub mod stack;

pub use compile::{CompileError, CompileErrorKind};
pub use runtime::{RuntimeError, RuntimeErrorKind};
pub use stack::StackFrame;

#[derive(Debug)]
pub enum AelysError {
    Compile(CompileError),
    Runtime(RuntimeError),
}

impl From<CompileError> for AelysError {
    fn from(e: CompileError) -> Self {
        AelysError::Compile(e)
    }
}

impl From<RuntimeError> for AelysError {
    fn from(e: RuntimeError) -> Self {
        AelysError::Runtime(e)
    }
}

impl fmt::Display for AelysError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            AelysError::Compile(e) => write!(f, "{}", e),
            AelysError::Runtime(e) => write!(f, "{}", e),
        }
    }
}

impl std::error::Error for AelysError {}

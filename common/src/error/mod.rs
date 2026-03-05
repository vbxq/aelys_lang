use std::fmt;

pub mod compile;
pub mod runtime;
pub mod stack;

use crate::diagnostic::Diagnostic;

pub use compile::{CompileError, CompileErrorKind};
pub use runtime::{RuntimeError, RuntimeErrorKind};
pub use stack::StackFrame;

#[derive(Debug)]
pub enum AelysError {
    Compile(CompileError),
    Runtime(RuntimeError),
    /// Multiple diagnostics (used when sema produces multiple independent errors)
    Multiple(Vec<Diagnostic>),
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
            AelysError::Multiple(diagnostics) => {
                for diag in diagnostics {
                    write!(f, "{}", diag)?;
                }
                Ok(())
            }
            _ => write!(f, "{}", self.to_diagnostic()),
        }
    }
}

impl std::error::Error for AelysError {}

impl AelysError {
    pub fn to_diagnostic(&self) -> Diagnostic {
        match self {
            AelysError::Compile(e) => e.to_diagnostic(),
            AelysError::Runtime(e) => e.to_diagnostic(),
            AelysError::Multiple(diagnostics) => {
                // return first diagnostic; callers should use to_diagnostics() instead
                diagnostics.first().cloned().unwrap_or_else(|| {
                    Diagnostic::new(crate::diagnostic::Severity::Error, "unknown error")
                })
            }
        }
    }

    /// Get all diagnostics from this error (for multi-error rendering)
    pub fn to_diagnostics(&self) -> Vec<Diagnostic> {
        match self {
            AelysError::Multiple(diagnostics) => diagnostics.clone(),
            _ => vec![self.to_diagnostic()],
        }
    }
}

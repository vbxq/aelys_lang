use std::fmt;

pub mod compile;
pub mod fault;

use crate::diagnostic::Diagnostic;

pub use compile::{CompileError, CompileErrorKind};
pub use fault::Fault;

#[derive(Debug)]
pub enum AelysError {
    Compile(CompileError),
    Multiple(Vec<Diagnostic>),
}

impl From<CompileError> for AelysError {
    fn from(e: CompileError) -> Self {
        AelysError::Compile(e)
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
            AelysError::Multiple(diagnostics) => {
                diagnostics.first().cloned().unwrap_or_else(|| {
                    Diagnostic::new(crate::diagnostic::Severity::Error, "unknown error")
                })
            }
        }
    }

    pub fn to_diagnostics(&self) -> Vec<Diagnostic> {
        match self {
            AelysError::Multiple(diagnostics) => diagnostics.clone(),
            _ => vec![self.to_diagnostic()],
        }
    }
}

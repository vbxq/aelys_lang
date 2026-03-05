pub mod diagnostic;
pub mod error;
pub mod result;
pub mod warning;

pub use diagnostic::{Diagnostic, Label, Severity};
pub use error::{
    AelysError, CompileError, CompileErrorKind, RuntimeError, RuntimeErrorKind, StackFrame,
};
pub use result::Result;
pub use warning::{Warning, WarningCollector, WarningConfig, WarningKind, format_warnings};

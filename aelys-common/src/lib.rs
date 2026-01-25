pub mod diagnostic;
pub mod error;
pub mod result;

pub use error::{
    AelysError, CompileError, CompileErrorKind, RuntimeError, RuntimeErrorKind, StackFrame,
};
pub use result::Result;

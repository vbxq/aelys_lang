//! Type constraints for the inference system.

mod definition;
mod error;
mod reason;

pub use definition::Constraint;
pub use error::{TypeError, TypeErrorKind, TypeErrorSuggestion};
pub use reason::ConstraintReason;

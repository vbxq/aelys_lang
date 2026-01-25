//! Unification algorithm for type inference.

mod convert;
mod error;
mod occurs;
mod substitution;
mod unify;

pub use convert::unify_error_to_type_error;
pub use error::{UnifyError, UnifyResult};
pub use substitution::Substitution;
pub use unify::unify;

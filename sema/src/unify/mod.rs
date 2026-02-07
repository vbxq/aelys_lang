//! Unification algorithm for type inference.

mod algorithm;
mod convert;
mod error;
mod occurs;
mod substitution;

pub use algorithm::unify;
pub use convert::unify_error_to_type_error;
pub use error::{UnifyError, UnifyResult};
pub use substitution::Substitution;

//! Core types for semantic analysis.

mod infer_type;
mod resolved_type;
mod type_var;

pub use infer_type::InferType;
pub use resolved_type::ResolvedType;
pub use type_var::{TypeVarGen, TypeVarId};

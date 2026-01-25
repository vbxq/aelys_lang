// HM-style inference with gradual typing fallback

mod captures;
mod constraints;
mod entry;
mod expr;
mod finalize;
mod functions;
mod lambda;
mod returns;
mod signatures;
mod stmt;
mod substitute;

use crate::constraint::{Constraint, TypeError};
use crate::env::TypeEnv;
use crate::types::{InferType, TypeVarGen};

/// Maximum recursion depth for type inference to prevent stack overflow
const MAX_INFERENCE_DEPTH: usize = 200;

/// The type inference engine
pub struct TypeInference {
    /// Type variable generator
    type_gen: TypeVarGen,

    /// Collected constraints
    constraints: Vec<Constraint>,

    /// Type environment
    env: TypeEnv,

    /// Collected type errors (warnings in gradual mode)
    errors: Vec<TypeError>,

    /// Stack of expected return types (for nested functions)
    return_type_stack: Vec<InferType>,

    /// Current recursion depth for preventing stack overflow
    depth: usize,
}

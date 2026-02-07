use crate::types::{InferType, TypeVarId};

/// Result of unification
pub type UnifyResult<T> = Result<T, UnifyError>;

/// Error during unification
#[derive(Debug, Clone)]
pub enum UnifyError {
    /// Types don't match
    Mismatch(InferType, InferType),
    /// Infinite type detected
    InfiniteType(TypeVarId, InferType),
    /// Function arity mismatch
    ArityMismatch(usize, usize),
}

impl std::fmt::Display for UnifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnifyError::Mismatch(t1, t2) => write!(f, "cannot unify {} with {}", t1, t2),
            UnifyError::InfiniteType(var, ty) => write!(f, "infinite type: {} = {}", var, ty),
            UnifyError::ArityMismatch(expected, found) => {
                write!(f, "arity mismatch: expected {}, found {}", expected, found)
            }
        }
    }
}

impl std::error::Error for UnifyError {}

use crate::types::{InferType, TypeVarId};

pub type UnifyResult<T> = Result<T, UnifyError>;

#[derive(Debug, Clone)]
pub enum UnifyError {
    Mismatch(InferType, InferType),
    InfiniteType(TypeVarId, InferType),
    ArityMismatch(usize, usize),
    /// a shared borrow reached a position requiring an exclusive one
    RefMutability(InferType, InferType),
}

impl std::fmt::Display for UnifyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            UnifyError::Mismatch(t1, t2) => write!(f, "cannot unify {} with {}", t1, t2),
            UnifyError::InfiniteType(var, ty) => write!(f, "infinite type: {} = {}", var, ty),
            UnifyError::ArityMismatch(expected, found) => {
                write!(f, "arity mismatch: expected {}, found {}", expected, found)
            }
            UnifyError::RefMutability(found, required) => {
                write!(f, "cannot use {} where {} is required", found, required)
            }
        }
    }
}

impl std::error::Error for UnifyError {}


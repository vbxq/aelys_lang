use crate::types::{InferType, TypeVarId};

/// Occurs check: does the variable occur in the type?
pub(super) fn occurs_check(var: TypeVarId, ty: &InferType) -> bool {
    match ty {
        InferType::Var(id) => *id == var,
        InferType::Function { params, ret } => {
            params.iter().any(|p| occurs_check(var, p)) || occurs_check(var, ret)
        }
        InferType::Array(inner) | InferType::Vec(inner) => occurs_check(var, inner),
        InferType::Tuple(elems) => elems.iter().any(|e| occurs_check(var, e)),
        InferType::Int
        | InferType::Float
        | InferType::Bool
        | InferType::String
        | InferType::Null
        | InferType::Range
        | InferType::Dynamic => false,
    }
}

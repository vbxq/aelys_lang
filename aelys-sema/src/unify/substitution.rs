use crate::types::{InferType, TypeVarId};
use std::collections::HashMap;

/// Substitution mapping type variables to types
#[derive(Debug, Clone, Default)]
pub struct Substitution {
    bindings: HashMap<TypeVarId, InferType>,
}

impl Substitution {
    pub fn new() -> Self {
        Self {
            bindings: HashMap::new(),
        }
    }

    /// Bind a type variable to a type
    pub fn bind(&mut self, var: TypeVarId, ty: InferType) {
        if ty != InferType::Var(var) {
            self.bindings.insert(var, ty);
        }
    }

    /// Check if a variable is bound
    pub fn is_bound(&self, var: TypeVarId) -> bool {
        self.bindings.contains_key(&var)
    }

    /// Get the binding for a variable
    pub fn get(&self, var: TypeVarId) -> Option<&InferType> {
        self.bindings.get(&var)
    }

    /// Apply the substitution to a type, resolving all bound variables
    pub fn apply(&self, ty: &InferType) -> InferType {
        match ty {
            InferType::Var(id) => {
                if let Some(bound) = self.bindings.get(id) {
                    self.apply(bound)
                } else {
                    ty.clone()
                }
            }
            InferType::Function { params, ret } => InferType::Function {
                params: params.iter().map(|p| self.apply(p)).collect(),
                ret: Box::new(self.apply(ret)),
            },
            InferType::Array(inner) => InferType::Array(Box::new(self.apply(inner))),
            InferType::Vec(inner) => InferType::Vec(Box::new(self.apply(inner))),
            InferType::Tuple(elems) => {
                InferType::Tuple(elems.iter().map(|e| self.apply(e)).collect())
            }
            InferType::Int
            | InferType::Float
            | InferType::Bool
            | InferType::String
            | InferType::Null
            | InferType::Range
            | InferType::Dynamic => ty.clone(),
        }
    }

    /// Compose two substitutions: self then other
    pub fn compose(&self, other: &Substitution) -> Substitution {
        let mut result = Substitution::new();

        for (var, ty) in &self.bindings {
            result.bindings.insert(*var, other.apply(ty));
        }

        for (var, ty) in &other.bindings {
            if !result.bindings.contains_key(var) {
                result.bindings.insert(*var, ty.clone());
            }
        }

        result
    }

    /// Get all bindings (for debugging)
    pub fn bindings(&self) -> &HashMap<TypeVarId, InferType> {
        &self.bindings
    }

    /// Number of bindings
    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    /// Check if empty
    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

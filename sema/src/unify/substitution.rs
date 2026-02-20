use crate::types::{InferType, TypeVarId};
use std::collections::HashMap;

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

    pub fn bind(&mut self, var: TypeVarId, ty: InferType) {
        if ty != InferType::Var(var) {
            self.bindings.insert(var, ty);
        }
    }

    pub fn is_bound(&self, var: TypeVarId) -> bool {
        self.bindings.contains_key(&var)
    }

    pub fn get(&self, var: TypeVarId) -> Option<&InferType> {
        self.bindings.get(&var)
    }

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
            InferType::I8
            | InferType::I16
            | InferType::I32
            | InferType::I64
            | InferType::U8
            | InferType::U16
            | InferType::U32
            | InferType::U64
            | InferType::F32
            | InferType::F64
            | InferType::Bool
            | InferType::String
            | InferType::Null
            | InferType::Range
            | InferType::Struct(_)
            | InferType::Dynamic => ty.clone(),
        }
    }

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

    pub fn bindings(&self) -> &HashMap<TypeVarId, InferType> {
        &self.bindings
    }

    pub fn len(&self) -> usize {
        self.bindings.len()
    }

    pub fn is_empty(&self) -> bool {
        self.bindings.is_empty()
    }
}

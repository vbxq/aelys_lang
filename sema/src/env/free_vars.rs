use super::TypeEnv;
use crate::types::{InferType, TypeVarId};
use std::collections::HashSet;

impl TypeEnv {
    /// Get free type variables in the environment
    pub fn free_type_vars(&self) -> HashSet<TypeVarId> {
        let mut vars = HashSet::new();

        fn collect_vars(ty: &InferType, vars: &mut HashSet<TypeVarId>) {
            match ty {
                InferType::Var(id) => {
                    vars.insert(*id);
                }
                InferType::Function { params, ret } => {
                    for p in params {
                        collect_vars(p, vars);
                    }
                    collect_vars(ret, vars);
                }
                InferType::Array(inner) => collect_vars(inner, vars),
                InferType::Tuple(elems) => {
                    for e in elems {
                        collect_vars(e, vars);
                    }
                }
                _ => {}
            }
        }

        for scope in &self.locals {
            for ty in scope.values() {
                collect_vars(ty, &mut vars);
            }
        }

        for ty in self.captures.values() {
            collect_vars(ty, &mut vars);
        }

        vars
    }
}

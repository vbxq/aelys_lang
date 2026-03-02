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
                InferType::Array(inner, _) => collect_vars(inner, vars),
                InferType::Vec(inner) => collect_vars(inner, vars),
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

// TODO: move them to aelys/tests when done
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn free_type_vars_finds_var_inside_vec() {
        let mut env = TypeEnv::new();
        // Vec(Var(7)), the Var inside the Vec should be collected
        env.define_local(
            "v".to_string(),
            InferType::Vec(Box::new(InferType::Var(TypeVarId(7)))),
        );
        let vars = env.free_type_vars();
        assert!(
            vars.contains(&TypeVarId(7)),
            "free_type_vars should descend into Vec element types"
        );
    }

    #[test]
    fn free_type_vars_finds_var_inside_array() {
        let mut env = TypeEnv::new();
        env.define_local(
            "a".to_string(),
            InferType::Array(Box::new(InferType::Var(TypeVarId(3))), Some(5)),
        );
        let vars = env.free_type_vars();
        assert!(
            vars.contains(&TypeVarId(3)),
            "free_type_vars should descend into Array element types"
        );
    }

    #[test]
    fn free_type_vars_finds_var_in_nested_vec() {
        let mut env = TypeEnv::new();
        // Vec(Vec(Var(9)))
        env.define_local(
            "nested".to_string(),
            InferType::Vec(Box::new(InferType::Vec(Box::new(InferType::Var(
                TypeVarId(9),
            ))))),
        );
        let vars = env.free_type_vars();
        assert!(
            vars.contains(&TypeVarId(9)),
            "free_type_vars should descend into nested Vec types"
        );
    }

    #[test]
    fn free_type_vars_ignores_concrete_vec() {
        let mut env = TypeEnv::new();
        env.define_local(
            "v".to_string(),
            InferType::Vec(Box::new(InferType::I64)),
        );
        let vars = env.free_type_vars();
        assert!(
            vars.is_empty(),
            "Vec(I64) has no free type variables"
        );
    }
}

use crate::types::{InferType, TypeVarId};
use std::collections::{HashMap, HashSet};

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
                    let mut visited = HashSet::new();
                    visited.insert(*id);
                    self.chase_var(bound, &mut visited)
                } else {
                    ty.clone()
                }
            }
            InferType::Function { params, ret } => InferType::Function {
                params: params.iter().map(|p| self.apply(p)).collect(),
                ret: Box::new(self.apply(ret)),
            },
            InferType::Array(inner, len) => InferType::Array(Box::new(self.apply(inner)), *len),
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

    /// chase a Var binding chain with cycle detection. If a Var is encountered that was already visited in this chain, return Dynamic to break the loop.
    fn chase_var(&self, ty: &InferType, visited: &mut HashSet<TypeVarId>) -> InferType {
        match ty {
            InferType::Var(id) => {
                if let Some(bound) = self.bindings.get(id) {
                    if !visited.insert(*id) {
                        return InferType::Dynamic;
                    }
                    self.chase_var(bound, visited)
                } else {
                    ty.clone()
                }
            }
            // onvr we reach a non-Var type, switch back to normal apply which starts fresh visited sets for any nested Vars
            other => self.apply(other),
        }
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

    /// Save a copy of the current bindings so we can roll back on failure.
    pub fn snapshot(&self) -> HashMap<TypeVarId, InferType> {
        self.bindings.clone()
    }

    /// Roll back the bindings to a previously saved state.
    pub fn restore(&mut self, saved: HashMap<TypeVarId, InferType>) {
        self.bindings = saved;
    }
}



// TODO: move that to dedicated aelys/tests
#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::InferType;
    use crate::types::TypeVarId;

    fn vid(n: u32) -> TypeVarId {
        TypeVarId(n)
    }

    #[test]
    fn apply_resolves_var_chain() {
        let mut subst = Substitution::new();
        // Var(0) -> Var(1) -> I64
        subst.bind(vid(0), InferType::Var(vid(1)));
        subst.bind(vid(1), InferType::I64);
        assert_eq!(subst.apply(&InferType::Var(vid(0))), InferType::I64);
    }

    #[test]
    fn apply_breaks_two_var_cycle() {
        let mut subst = Substitution::new();
        // fo a cycle by manually inserting: Var(0) -> Var(1) -> Var(0)
        subst.bindings.insert(vid(0), InferType::Var(vid(1)));
        subst.bindings.insert(vid(1), InferType::Var(vid(0)));
        // Without cycle detection this would stack overflow, with cycle detection it returns Dynamic.
        let result = subst.apply(&InferType::Var(vid(0)));
        assert_eq!(result, InferType::Dynamic);
    }

    #[test]
    fn apply_breaks_three_var_cycle() {
        let mut subst = Substitution::new();
        // Var(0) -> Var(1) -> Var(2) -> Var(0)
        subst.bindings.insert(vid(0), InferType::Var(vid(1)));
        subst.bindings.insert(vid(1), InferType::Var(vid(2)));
        subst.bindings.insert(vid(2), InferType::Var(vid(0)));
        let result = subst.apply(&InferType::Var(vid(0)));
        assert_eq!(result, InferType::Dynamic);
    }

    #[test]
    fn apply_no_false_positive_on_diamond() {
        let mut subst = Substitution::new();
        // Var(0) -> I64, Var(1) -> I64 (same target, not a cycle)
        subst.bind(vid(0), InferType::I64);
        subst.bind(vid(1), InferType::I64);
        // function with two params both referencing the same resolved type
        let fn_ty = InferType::Function {
            params: vec![InferType::Var(vid(0)), InferType::Var(vid(1))],
            ret: Box::new(InferType::Var(vid(0))),
        };
        let result = subst.apply(&fn_ty);
        assert_eq!(
            result,
            InferType::Function {
                params: vec![InferType::I64, InferType::I64],
                ret: Box::new(InferType::I64),
            }
        );
    }
}

use super::TypeInference;
use crate::constraint::{Constraint, TypeError};
use crate::types::InferType;
use crate::unify::{Substitution, unify, unify_error_to_type_error};

impl TypeInference {
    /// Solve all collected constraints with gradual fallback
    pub(super) fn solve_constraints(&mut self) -> Substitution {
        let mut subst = Substitution::new();

        for constraint in self.constraints.clone() {
            if let Constraint::Equal {
                left,
                right,
                span,
                reason,
            } = constraint
            {
                let left_resolved = subst.apply(&left);
                let right_resolved = subst.apply(&right);

                let saved = subst.snapshot();
                match unify(&left_resolved, &right_resolved, &mut subst) {
                    Ok(()) => {}
                    Err(e) => {
                        // roll back any partial bindings from sub-unifications that succeeded before this one failed
                        subst.restore(saved);

                        let err = unify_error_to_type_error(e, span, reason);
                        self.errors.push(err);

                        self.force_dynamic(&left_resolved, &mut subst);
                        self.force_dynamic(&right_resolved, &mut subst);
                    }
                }
            }
        }

        for constraint in self.constraints.clone() {
            if let Constraint::OneOf {
                ty,
                options,
                span,
                reason,
            } = constraint
            {
                let resolved = subst.apply(&ty);

                match &resolved {
                    InferType::Dynamic => {}
                    InferType::Var(id) => {
                        // if the Var is still unresolved after the Equal pass, bind it to the widest type in the option set
                        // this lets OneOf constraints participate in inference instead of letting unresolved Vars fall through to Dynamic via finalization
                        if let Some(default) = Self::pick_widest_type(&options) {
                            subst.bind(*id, default);
                        }
                    }
                    concrete => {
                        let mut matched = false;
                        for opt in &options {
                            let mut temp_subst = subst.clone();
                            if unify(concrete, opt, &mut temp_subst).is_ok() {
                                // adopt the entire temp_subst so that all bindings from the successful unification are applied atomically.
                                //
                                // ! the old merge-by-key pattern (`if !subst.is_bound`) could leak stale bindings when earlier failed options partially bound vars that the successful option did not touch.
                                subst = temp_subst;
                                matched = true;
                                break;
                            }
                            // failed temp_subst is simply dropped, subst is untouched
                        }

                        if !matched {
                            self.errors.push(TypeError::not_one_of(
                                concrete.clone(),
                                options.clone(),
                                span,
                                reason,
                            ));
                            self.force_dynamic(&resolved, &mut subst);
                        }
                    }
                }
            }
        }

        subst
    }

    /// Pick the widest type from a set of options for defaulting an unresolved type variable constrained by OneOf.
    ///
    /// returns i64 if the set contains integer types, f64 if it contains only float types.
    /// returns None for non-numeric sets.
    fn pick_widest_type(options: &[InferType]) -> Option<InferType> {
        let has_integers = options.iter().any(|t| t.is_integer());
        let has_floats = options.iter().any(|t| t.is_float());

        if has_integers {
            Some(InferType::I64)
        } else if has_floats {
            Some(InferType::F64)
        } else {
            None
        }
    }

    /// force a type to Dynamic (for error recovery)
    ///
    /// only binds top-level Vars. i deliberately avoid recursing into compound types (Function, Array, Vec, Tuple) because their inner Vars may
    /// be shared with unrelated constraints. binding those would silently corrupt types in expressions that had no errors.
    fn force_dynamic(&mut self, ty: &InferType, subst: &mut Substitution) {
        if let InferType::Var(id) = ty {
            subst.bind(*id, InferType::Dynamic);
        }
    }
}

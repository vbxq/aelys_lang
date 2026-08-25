use super::TypeInference;
use crate::constraint::{Constraint, TypeError};
use crate::types::InferType;
use crate::unify::{Substitution, unify, unify_dir, unify_error_to_type_error};

impl TypeInference {
    pub(super) fn solve_constraints(&mut self) -> Substitution {
        let mut subst = Substitution::new();

        for constraint in self.constraints.clone() {
            if let Constraint::Equal {
                left,
                right,
                span,
                reason,
                dir,
            } = constraint
            {
                let left_resolved = subst.apply(&left);
                let right_resolved = subst.apply(&right);

                let saved = subst.snapshot();
                match unify_dir(&left_resolved, &right_resolved, &mut subst, dir) {
                    Ok(()) => {}
                    Err(e) => {
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
                        if let Some(default) = Self::pick_widest_type(&options) {
                            subst.bind(*id, default);
                        }
                    }
                    concrete => {
                        let mut matched = false;
                        for opt in &options {
                            let mut temp_subst = subst.clone();
                            if unify(concrete, opt, &mut temp_subst).is_ok() {
                                // ! the old merge-by-key pattern (`if !subst.is_bound`) could leak stale bindings when earlier failed options partially bound vars that the successful option did not touch.
                                subst = temp_subst;
                                matched = true;
                                break;
                            }
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

    fn force_dynamic(&mut self, ty: &InferType, subst: &mut Substitution) {
        if let InferType::Var(id) = ty {
            subst.bind(*id, InferType::Dynamic);
        }
    }
}

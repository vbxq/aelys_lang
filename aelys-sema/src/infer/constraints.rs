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

                match unify(&left_resolved, &right_resolved, &mut subst) {
                    Ok(()) => {}
                    Err(e) => {
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
                    InferType::Var(_) | InferType::Dynamic => {}
                    concrete => {
                        let matches = options.iter().any(|opt| {
                            let mut temp_subst = subst.clone();
                            unify(concrete, opt, &mut temp_subst).is_ok()
                        });

                        if !matches {
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

    /// Force a type to Dynamic (for error recovery)
    fn force_dynamic(&mut self, ty: &InferType, subst: &mut Substitution) {
        match ty {
            InferType::Var(id) => {
                subst.bind(*id, InferType::Dynamic);
            }
            InferType::Function { params, ret } => {
                for p in params {
                    self.force_dynamic(p, subst);
                }
                self.force_dynamic(ret, subst);
            }
            InferType::Array(inner) => {
                self.force_dynamic(inner, subst);
            }
            InferType::Tuple(elems) => {
                for e in elems {
                    self.force_dynamic(e, subst);
                }
            }
            _ => {}
        }
    }
}

use super::TypeInference;
use crate::constraint::{Constraint, TypeError};
use crate::types::InferType;
use crate::unify::{Substitution, unify, unify_dir, unify_error_to_type_error};

impl TypeInference {
    fn recover_param_name(&self, ty: &InferType) -> InferType {
        match ty {
            InferType::Var(id) => match self.instantiated_param_names.get(id) {
                Some(name) => InferType::Struct(name.clone()),
                None => ty.clone(),
            },
            InferType::Array(inner, len) => {
                InferType::Array(Box::new(self.recover_param_name(inner)), *len)
            }
            InferType::Vec(inner) => InferType::Vec(Box::new(self.recover_param_name(inner))),
            InferType::Rc(inner) => InferType::Rc(Box::new(self.recover_param_name(inner))),
            InferType::Ref { referent, mutable } => InferType::Ref {
                referent: Box::new(self.recover_param_name(referent)),
                mutable: *mutable,
            },
            InferType::Slice { elem, mutable } => InferType::Slice {
                elem: Box::new(self.recover_param_name(elem)),
                mutable: *mutable,
            },
            InferType::Tuple(elems) => {
                InferType::Tuple(elems.iter().map(|e| self.recover_param_name(e)).collect())
            }
            InferType::Enum(name, args) => InferType::Enum(
                name.clone(),
                args.iter().map(|a| self.recover_param_name(a)).collect(),
            ),
            InferType::Function { params, ret, nogc } => InferType::Function {
                params: params.iter().map(|p| self.recover_param_name(p)).collect(),
                ret: Box::new(self.recover_param_name(ret)),
                nogc: *nogc,
            },
            other => other.clone(),
        }
    }

    fn name_the_type_params(&self, mut err: TypeError) -> TypeError {
        use crate::constraint::TypeErrorKind;
        err.kind = match err.kind {
            TypeErrorKind::Mismatch { expected, found } => TypeErrorKind::Mismatch {
                expected: self.recover_param_name(&expected),
                found: self.recover_param_name(&found),
            },
            TypeErrorKind::RefMutability { found, required } => TypeErrorKind::RefMutability {
                found: self.recover_param_name(&found),
                required: self.recover_param_name(&required),
            },
            other => other,
        };
        err
    }

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

                        let err = self
                            .name_the_type_params(unify_error_to_type_error(e, span, reason, dir));
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
                                // ! the old merge-by-key pattern leaked stale bindings from earlier failed options
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

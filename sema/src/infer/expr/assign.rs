use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError};
use crate::typed_ast::TypedExprKind;
use crate::types::InferType;
use aelys_syntax::{Expr, Span};

impl TypeInference {
    pub(super) fn infer_assign_expr(
        &mut self,
        name: &str,
        value: &Expr,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let mut typed_value = self.infer_expr(value);

        if let Some(var_type) = self.env.lookup(name).cloned() {
            // check mutability, reject assignment to immutable variables
            if !self.env.is_mutable(name) {
                self.errors.push(TypeError {
                    kind: crate::constraint::TypeErrorKind::Mismatch {
                        expected: var_type.clone(),
                        found: typed_value.ty.clone(),
                    },
                    span,
                    reason: ConstraintReason::Other(format!(
                        "cannot assign to immutable variable '{}' (use 'let mut' to make it mutable)",
                        name
                    )),
                });
            }

            self.try_narrow_literal(&mut typed_value, &var_type);

            self.constraints.push(Constraint::equal(
                typed_value.ty.clone(),
                var_type.clone(),
                span,
                ConstraintReason::Assignment {
                    var_name: name.to_string(),
                },
            ));

            (
                TypedExprKind::Assign {
                    name: name.to_string(),
                    value: Box::new(typed_value),
                },
                InferType::Null,
            )
        } else {
            self.errors
                .push(TypeError::undefined_variable(name.to_string(), span));
            (
                TypedExprKind::Assign {
                    name: name.to_string(),
                    value: Box::new(typed_value),
                },
                InferType::Null,
            )
        }
    }
}

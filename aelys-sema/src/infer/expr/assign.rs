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
        let typed_value = self.infer_expr(value);

        if let Some(var_type) = self.env.lookup(name).cloned() {
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
                var_type,
            )
        } else {
            self.errors
                .push(TypeError::undefined_variable(name.to_string(), span));
            (
                TypedExprKind::Assign {
                    name: name.to_string(),
                    value: Box::new(typed_value),
                },
                InferType::Dynamic,
            )
        }
    }
}

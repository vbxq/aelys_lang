use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorSuggestion};
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
                let binding_span = self.env.lookup_binding_span(name);
                let suggestion = binding_span.map(|bs| {
                    // create a zero-width insertion span right after `let `, the binding_span starts at `let`, so column + 4 is where the variable name begins, we insert `mut ` there
                    let insert_offset = bs.start + 4; // skip "let "
                    let insert_span = Span::new(insert_offset, insert_offset, bs.line, bs.column + 4);
                    TypeErrorSuggestion {
                        message: "make the binding mutable".to_string(),
                        span: insert_span,
                        new_text: "mut ".to_string(),
                    }
                });
                self.errors.push(TypeError::assign_to_immutable(
                    name.to_string(),
                    span,
                    binding_span,
                    suggestion,
                ));
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

            // register the variable with Dynamic type to prevent repeated "undefined variable" errors for each subsequent use
            self.env.define_local(name.to_string(), InferType::Dynamic);

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

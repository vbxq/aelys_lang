use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::{Expr, Span};

impl TypeInference {
    pub(super) fn infer_return_stmt(&mut self, span: Span, expr: Option<&Expr>) -> TypedStmtKind {
        let mut typed_expr = expr.map(|e| self.infer_expr(e));

        if let Some(expected_ret) = self.current_return_type().cloned() {
            if let Some(ref mut texpr) = typed_expr {
                self.try_narrow_literal(texpr, &expected_ret);

                // Implicit numeric widening (e.g. return i32_val from fn -> i64)
                if texpr.ty != expected_ret
                    && texpr.ty.can_implicit_widen_to(&expected_ret)
                {
                    let vspan = texpr.span;
                    let original = std::mem::replace(
                        texpr,
                        TypedExpr { kind: TypedExprKind::Null, ty: InferType::Null, span: vspan },
                    );
                    *texpr = TypedExpr {
                        kind: TypedExprKind::Cast {
                            expr: Box::new(original),
                            target: expected_ret.clone(),
                        },
                        ty: expected_ret.clone(),
                        span: vspan,
                    };
                }
            }

            let actual_ret = typed_expr
                .as_ref()
                .map(|e| e.ty.clone())
                .unwrap_or(InferType::Null);

            self.constraints.push(Constraint::equal(
                expected_ret,
                actual_ret,
                span,
                ConstraintReason::Return {
                    func_name: self
                        .env
                        .current_function()
                        .cloned()
                        .unwrap_or_else(|| "<anonymous>".to_string()),
                },
            ));
        }

        TypedStmtKind::Return(typed_expr)
    }
}

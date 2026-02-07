use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::TypedStmtKind;
use crate::types::InferType;
use aelys_syntax::{Expr, Span};

impl TypeInference {
    pub(super) fn infer_return_stmt(&mut self, span: Span, expr: Option<&Expr>) -> TypedStmtKind {
        let typed_expr = expr.map(|e| self.infer_expr(e));

        if let Some(expected_ret) = self.current_return_type().cloned() {
            let actual_ret = typed_expr
                .as_ref()
                .map(|e| e.ty.clone())
                .unwrap_or(InferType::Null);

            self.constraints.push(Constraint::equal(
                actual_ret,
                expected_ret,
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

use super::TypeInference;
use crate::typed_ast::TypedExprKind;
use aelys_syntax::{Parameter, Span, Stmt, TypeAnnotation};

impl TypeInference {
    pub(super) fn infer_lambda_expr(
        &mut self,
        params: &[Parameter],
        return_type: Option<&TypeAnnotation>,
        body: &[Stmt],
        span: Span,
    ) -> (TypedExprKind, crate::types::InferType) {
        let typed_lambda = self.infer_lambda(params, return_type, body, span);
        let ty = typed_lambda.ty.clone();
        (TypedExprKind::Lambda(Box::new(typed_lambda)), ty)
    }
}

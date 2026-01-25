use super::TypeInference;
use crate::typed_ast::TypedExprKind;
use crate::types::InferType;
use aelys_syntax::{Expr, Span};

impl TypeInference {
    pub(super) fn infer_member_expr(
        &mut self,
        object: &Expr,
        member: &str,
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_object = self.infer_expr(object);
        (
            TypedExprKind::Member {
                object: Box::new(typed_object),
                member: member.to_string(),
            },
            InferType::Dynamic,
        )
    }
}

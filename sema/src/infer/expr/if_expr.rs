use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::TypedExprKind;
use crate::types::InferType;
use aelys_syntax::{Expr, Span};

impl TypeInference {
    pub(super) fn infer_if_expr(
        &mut self,
        condition: &Expr,
        then_branch: &Expr,
        else_branch: &Expr,
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_cond = self.infer_expr(condition);
        let typed_then = self.infer_expr(then_branch);
        let typed_else = self.infer_expr(else_branch);

        self.constraints.push(Constraint::equal(
            typed_cond.ty.clone(),
            InferType::Bool,
            condition.span,
            ConstraintReason::IfCondition,
        ));

        let result_type = self.type_gen.fresh();
        self.constraints.push(Constraint::equal(
            typed_then.ty.clone(),
            result_type.clone(),
            then_branch.span,
            ConstraintReason::IfBranches,
        ));
        self.constraints.push(Constraint::equal(
            typed_else.ty.clone(),
            result_type.clone(),
            else_branch.span,
            ConstraintReason::IfBranches,
        ));

        (
            TypedExprKind::If {
                condition: Box::new(typed_cond),
                then_branch: Box::new(typed_then),
                else_branch: Box::new(typed_else),
            },
            result_type,
        )
    }
}

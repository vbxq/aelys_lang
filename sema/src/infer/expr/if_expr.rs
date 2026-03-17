use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::{TypedExpr, TypedExprKind};
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
        let mut typed_then = self.infer_expr(then_branch);
        let mut typed_else = self.infer_expr(else_branch);

        self.constraints.push(Constraint::equal(
            typed_cond.ty.clone(),
            InferType::Bool,
            condition.span,
            ConstraintReason::IfCondition,
        ));

        // Implicit numeric widening between branches: if one branch is
        // a wider numeric type, widen the other (e.g. i32 vs i64 → both i64).
        if typed_then.ty != typed_else.ty
            && typed_then.ty.can_implicit_widen_to(&typed_else.ty)
        {
            let span = typed_then.span;
            let original = std::mem::replace(
                &mut typed_then,
                TypedExpr { kind: TypedExprKind::Null, ty: InferType::Null, span },
            );
            typed_then = TypedExpr {
                kind: TypedExprKind::Cast {
                    expr: Box::new(original),
                    target: typed_else.ty.clone(),
                },
                ty: typed_else.ty.clone(),
                span,
            };
        } else if typed_then.ty != typed_else.ty
            && typed_else.ty.can_implicit_widen_to(&typed_then.ty)
        {
            let span = typed_else.span;
            let original = std::mem::replace(
                &mut typed_else,
                TypedExpr { kind: TypedExprKind::Null, ty: InferType::Null, span },
            );
            typed_else = TypedExpr {
                kind: TypedExprKind::Cast {
                    expr: Box::new(original),
                    target: typed_then.ty.clone(),
                },
                ty: typed_then.ty.clone(),
                span,
            };
        }

        // when both branches have the same concrete type, use it directly instead of creating a fresh Var.
        // this is what infer_binary_op does and allows downstream narrowing to see the real type
        let result_type = if typed_then.ty == typed_else.ty && typed_then.ty.is_concrete() {
            typed_then.ty.clone()
        } else if let (InferType::Enum(a, _), InferType::Enum(b, _)) =
            (&typed_then.ty, &typed_else.ty)
        {
            if a == b {
                // same enum name but different type var args (e.g. Enum("Option", [Var(1)]) vs Enum("Option", [Var(2)])).
                // use the then-branch type and constrain both to unify.
                self.constraints.push(Constraint::equal(
                    typed_then.ty.clone(),
                    typed_else.ty.clone(),
                    else_branch.span,
                    ConstraintReason::IfBranches,
                ));
                typed_then.ty.clone()
            } else {
                let fresh = self.type_gen.fresh();
                self.constraints.push(Constraint::equal(
                    typed_then.ty.clone(),
                    fresh.clone(),
                    then_branch.span,
                    ConstraintReason::IfBranches,
                ));
                self.constraints.push(Constraint::equal(
                    typed_else.ty.clone(),
                    fresh.clone(),
                    else_branch.span,
                    ConstraintReason::IfBranches,
                ));
                fresh
            }
        } else {
            let fresh = self.type_gen.fresh();
            self.constraints.push(Constraint::equal(
                typed_then.ty.clone(),
                fresh.clone(),
                then_branch.span,
                ConstraintReason::IfBranches,
            ));
            self.constraints.push(Constraint::equal(
                typed_else.ty.clone(),
                fresh.clone(),
                else_branch.span,
                ConstraintReason::IfBranches,
            ));
            fresh
        };

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

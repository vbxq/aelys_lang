use super::super::TypeInference;
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedParam};
use crate::unify::Substitution;

impl TypeInference {
    /// Apply substitution to an expression
    pub(super) fn apply_substitution_expr(
        &self,
        expr: &TypedExpr,
        subst: &Substitution,
    ) -> TypedExpr {
        let kind = match &expr.kind {
            TypedExprKind::Int(n) => TypedExprKind::Int(*n),
            TypedExprKind::Float(f) => TypedExprKind::Float(*f),
            TypedExprKind::Bool(b) => TypedExprKind::Bool(*b),
            TypedExprKind::String(s) => TypedExprKind::String(s.clone()),
            TypedExprKind::Null => TypedExprKind::Null,
            TypedExprKind::Identifier(name) => TypedExprKind::Identifier(name.clone()),
            TypedExprKind::Binary { left, op, right } => TypedExprKind::Binary {
                left: Box::new(self.apply_substitution_expr(left, subst)),
                op: *op,
                right: Box::new(self.apply_substitution_expr(right, subst)),
            },
            TypedExprKind::Unary { op, operand } => TypedExprKind::Unary {
                op: *op,
                operand: Box::new(self.apply_substitution_expr(operand, subst)),
            },
            TypedExprKind::And { left, right } => TypedExprKind::And {
                left: Box::new(self.apply_substitution_expr(left, subst)),
                right: Box::new(self.apply_substitution_expr(right, subst)),
            },
            TypedExprKind::Or { left, right } => TypedExprKind::Or {
                left: Box::new(self.apply_substitution_expr(left, subst)),
                right: Box::new(self.apply_substitution_expr(right, subst)),
            },
            TypedExprKind::Call { callee, args } => TypedExprKind::Call {
                callee: Box::new(self.apply_substitution_expr(callee, subst)),
                args: args
                    .iter()
                    .map(|a| self.apply_substitution_expr(a, subst))
                    .collect(),
            },
            TypedExprKind::Assign { name, value } => TypedExprKind::Assign {
                name: name.clone(),
                value: Box::new(self.apply_substitution_expr(value, subst)),
            },
            TypedExprKind::Grouping(inner) => {
                TypedExprKind::Grouping(Box::new(self.apply_substitution_expr(inner, subst)))
            }
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => TypedExprKind::If {
                condition: Box::new(self.apply_substitution_expr(condition, subst)),
                then_branch: Box::new(self.apply_substitution_expr(then_branch, subst)),
                else_branch: Box::new(self.apply_substitution_expr(else_branch, subst)),
            },
            TypedExprKind::Lambda(inner) => {
                TypedExprKind::Lambda(Box::new(self.apply_substitution_expr(inner, subst)))
            }
            TypedExprKind::LambdaInner {
                params,
                return_type,
                body,
                captures,
            } => TypedExprKind::LambdaInner {
                params: params
                    .iter()
                    .map(|p| TypedParam {
                        name: p.name.clone(),
                        ty: subst.apply(&p.ty),
                        span: p.span,
                    })
                    .collect(),
                return_type: subst.apply(return_type),
                body: body
                    .iter()
                    .map(|s| self.apply_substitution_stmt(s, subst))
                    .collect(),
                captures: captures
                    .iter()
                    .map(|(name, ty)| (name.clone(), subst.apply(ty)))
                    .collect(),
            },
            TypedExprKind::Member { object, member } => TypedExprKind::Member {
                object: Box::new(self.apply_substitution_expr(object, subst)),
                member: member.clone(),
            },
        };

        TypedExpr {
            kind,
            ty: subst.apply(&expr.ty),
            span: expr.span,
        }
    }
}

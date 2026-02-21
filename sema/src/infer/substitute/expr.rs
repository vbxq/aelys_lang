use super::super::TypeInference;
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedFmtStringPart, TypedParam};
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
            TypedExprKind::FmtString(parts) => TypedExprKind::FmtString(
                parts
                    .iter()
                    .map(|p| match p {
                        TypedFmtStringPart::Literal(s) => TypedFmtStringPart::Literal(s.clone()),
                        TypedFmtStringPart::Expr(e) => TypedFmtStringPart::Expr(Box::new(
                            self.apply_substitution_expr(e, subst),
                        )),
                        TypedFmtStringPart::Placeholder => TypedFmtStringPart::Placeholder,
                    })
                    .collect(),
            ),
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
                        mutable: p.mutable,
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
            TypedExprKind::ArrayLiteral {
                element_type,
                elements,
            } => TypedExprKind::ArrayLiteral {
                element_type: element_type.clone(),
                elements: elements
                    .iter()
                    .map(|e| self.apply_substitution_expr(e, subst))
                    .collect(),
            },
            TypedExprKind::ArraySized { element_type, size } => TypedExprKind::ArraySized {
                element_type: element_type.clone(),
                size: Box::new(self.apply_substitution_expr(size, subst)),
            },
            TypedExprKind::VecLiteral {
                element_type,
                elements,
            } => TypedExprKind::VecLiteral {
                element_type: element_type.clone(),
                elements: elements
                    .iter()
                    .map(|e| self.apply_substitution_expr(e, subst))
                    .collect(),
            },
            TypedExprKind::Index { object, index } => TypedExprKind::Index {
                object: Box::new(self.apply_substitution_expr(object, subst)),
                index: Box::new(self.apply_substitution_expr(index, subst)),
            },
            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => TypedExprKind::IndexAssign {
                object: Box::new(self.apply_substitution_expr(object, subst)),
                index: Box::new(self.apply_substitution_expr(index, subst)),
                value: Box::new(self.apply_substitution_expr(value, subst)),
            },
            TypedExprKind::Range {
                start,
                end,
                inclusive,
            } => TypedExprKind::Range {
                start: start
                    .as_ref()
                    .map(|s| Box::new(self.apply_substitution_expr(s, subst))),
                end: end
                    .as_ref()
                    .map(|e| Box::new(self.apply_substitution_expr(e, subst))),
                inclusive: *inclusive,
            },
            TypedExprKind::Slice { object, range } => TypedExprKind::Slice {
                object: Box::new(self.apply_substitution_expr(object, subst)),
                range: Box::new(self.apply_substitution_expr(range, subst)),
            },
            TypedExprKind::StructLiteral { name, fields } => TypedExprKind::StructLiteral {
                name: name.clone(),
                fields: fields
                    .iter()
                    .map(|(n, v)| (n.clone(), Box::new(self.apply_substitution_expr(v, subst))))
                    .collect(),
            },
            TypedExprKind::Cast { expr, target } => TypedExprKind::Cast {
                expr: Box::new(self.apply_substitution_expr(expr, subst)),
                target: subst.apply(target),
            },
        };

        TypedExpr {
            kind,
            ty: subst.apply(&expr.ty),
            span: expr.span,
        }
    }
}

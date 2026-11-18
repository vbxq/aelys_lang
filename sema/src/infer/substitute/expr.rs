use super::super::TypeInference;
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedFmtStringPart, TypedParam};
use crate::unify::Substitution;

impl TypeInference {
    pub(super) fn apply_substitution_expr(
        &self,
        expr: &TypedExpr,
        subst: &Substitution,
    ) -> TypedExpr {
        let kind = match &expr.kind {
            TypedExprKind::Int(n) => TypedExprKind::Int(*n),
            TypedExprKind::Float(f) => TypedExprKind::Float(*f),
            TypedExprKind::Bool(b) => TypedExprKind::Bool(*b),
            TypedExprKind::Char(cp) => TypedExprKind::Char(*cp),
            TypedExprKind::String(s) => TypedExprKind::String(s.clone()),
            TypedExprKind::FmtString(parts) => {
                let parts = parts
                    .iter()
                    .map(|p| match p {
                        TypedFmtStringPart::Literal(s) => TypedFmtStringPart::Literal(s.clone()),
                        TypedFmtStringPart::Expr(e) => TypedFmtStringPart::Expr(Box::new(
                            self.apply_substitution_expr(e, subst),
                        )),
                        TypedFmtStringPart::Placeholder => TypedFmtStringPart::Placeholder,
                    })
                    .collect();
                match self.tuple_format(parts, expr.span) {
                    Ok(desugared) => return desugared,
                    Err(parts) => TypedExprKind::FmtString(parts),
                }
            }
            TypedExprKind::Null => TypedExprKind::Null,
            TypedExprKind::Identifier(name) => TypedExprKind::Identifier(name.clone()),
            TypedExprKind::Binary { left, op, right } => {
                let left = self.apply_substitution_expr(left, subst);
                let right = self.apply_substitution_expr(right, subst);
                if matches!(left.ty, crate::types::InferType::Tuple(_)) {
                    if let Some(desugared) =
                        self.tuple_equality(left.clone(), *op, right.clone(), expr.span)
                    {
                        return desugared;
                    }
                }
                TypedExprKind::Binary {
                    left: Box::new(left),
                    op: *op,
                    right: Box::new(right),
                }
            }
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
            TypedExprKind::Call { callee, args } => {
                let prints = matches!(&callee.kind,
                    TypedExprKind::Identifier(n) if n == "println" || n == "print");
                TypedExprKind::Call {
                    callee: Box::new(self.apply_substitution_expr(callee, subst)),
                    args: args
                        .iter()
                        .map(|a| {
                            let a = self.apply_substitution_expr(a, subst);
                            if prints { self.tuple_print_arg(a) } else { a }
                        })
                        .collect(),
                }
            }
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
            TypedExprKind::ArrayLiteral { elements } => TypedExprKind::ArrayLiteral {
                elements: elements
                    .iter()
                    .map(|e| self.apply_substitution_expr(e, subst))
                    .collect(),
            },
            TypedExprKind::ArraySized { size, fill_value } => TypedExprKind::ArraySized {
                size: Box::new(self.apply_substitution_expr(size, subst)),
                fill_value: fill_value
                    .as_ref()
                    .map(|fv| Box::new(self.apply_substitution_expr(fv, subst))),
            },
            TypedExprKind::VecLiteral {
                element_type: _,
                elements,
            } => {
                let substituted_ty = subst.apply(&expr.ty);
                let new_elem_type = match &substituted_ty {
                    crate::types::InferType::Vec(inner) => {
                        Some(crate::types::ResolvedType::from_infer_type(inner))
                    }
                    _ => None,
                };
                TypedExprKind::VecLiteral {
                    element_type: new_elem_type,
                    elements: elements
                        .iter()
                        .map(|e| self.apply_substitution_expr(e, subst))
                        .collect(),
                }
            }
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
            TypedExprKind::FieldAssign {
                object,
                field,
                value,
            } => TypedExprKind::FieldAssign {
                object: Box::new(self.apply_substitution_expr(object, subst)),
                field: field.clone(),
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
            TypedExprKind::Reference {
                mutable,
                operand,
                receiver_of,
            } => TypedExprKind::Reference {
                mutable: *mutable,
                operand: Box::new(self.apply_substitution_expr(operand, subst)),
                receiver_of: receiver_of.clone(),
            },
            TypedExprKind::Deref(operand) => {
                TypedExprKind::Deref(Box::new(self.apply_substitution_expr(operand, subst)))
            }
            TypedExprKind::DerefAssign { target, value } => TypedExprKind::DerefAssign {
                target: Box::new(self.apply_substitution_expr(target, subst)),
                value: Box::new(self.apply_substitution_expr(value, subst)),
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
            TypedExprKind::EnumVariant {
                enum_name,
                variant,
                tag,
                args,
            } => TypedExprKind::EnumVariant {
                enum_name: enum_name.clone(),
                variant: variant.clone(),
                tag: *tag,
                args: args
                    .iter()
                    .map(|a| self.apply_substitution_expr(a, subst))
                    .collect(),
            },
            TypedExprKind::Block { stmts, tail } => TypedExprKind::Block {
                stmts: stmts
                    .iter()
                    .map(|s| self.apply_substitution_stmt(s, subst))
                    .collect(),
                tail: Box::new(self.apply_substitution_expr(tail, subst)),
            },
            TypedExprKind::Loop(body) => {
                TypedExprKind::Loop(Box::new(self.apply_substitution_stmt(body, subst)))
            }
            TypedExprKind::Match {
                scrutinee,
                arms,
                origin,
            } => TypedExprKind::Match {
                scrutinee: Box::new(self.apply_substitution_expr(scrutinee, subst)),
                arms: arms
                    .iter()
                    .map(|arm| {
                        let mut arm = arm
                            .clone()
                            .map_exprs(|e| self.apply_substitution_expr(&e, subst));
                        arm.pattern = arm.pattern.map_types(&mut |t| subst.apply(&t));
                        arm
                    })
                    .collect(),
                origin: *origin,
            },

            TypedExprKind::ResultAssert {
                scrutinee,
                ok_tag,
                payload_ty,
                on_err,
            } => TypedExprKind::ResultAssert {
                scrutinee: Box::new(self.apply_substitution_expr(scrutinee, subst)),
                ok_tag: *ok_tag,
                payload_ty: subst.apply(payload_ty),
                on_err: on_err.clone(),
            },
        };

        TypedExpr {
            kind,
            ty: subst.apply(&expr.ty),
            span: expr.span,
        }
    }
}

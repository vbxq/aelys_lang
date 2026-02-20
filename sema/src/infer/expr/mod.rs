mod array;
mod assign;
mod binary;
mod call;
mod if_expr;
mod lambda;
mod member;
mod primary;

use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedFmtStringPart};
use crate::types::InferType;
use aelys_syntax::{Expr, ExprKind};

impl TypeInference {
    /// Infer type for an expression
    pub(super) fn infer_expr(&mut self, expr: &Expr) -> TypedExpr {
        self.depth += 1;
        if self.depth > super::MAX_INFERENCE_DEPTH {
            self.errors.push(TypeError::recursion_limit(expr.span));
            self.depth -= 1;
            return TypedExpr {
                kind: TypedExprKind::Null,
                ty: InferType::Dynamic,
                span: expr.span,
            };
        }

        let (kind, ty) = match &expr.kind {
            ExprKind::Int(n) => (TypedExprKind::Int(*n), InferType::I64),
            ExprKind::Float(f) => (TypedExprKind::Float(*f), InferType::F64),
            ExprKind::Bool(b) => (TypedExprKind::Bool(*b), InferType::Bool),
            ExprKind::String(s) => (TypedExprKind::String(s.clone()), InferType::String),
            ExprKind::FmtString(parts) => {
                let typed_parts = parts
                    .iter()
                    .map(|p| match p {
                        aelys_syntax::FmtStringPart::Literal(s) => {
                            TypedFmtStringPart::Literal(s.clone())
                        }
                        aelys_syntax::FmtStringPart::Expr(e) => {
                            TypedFmtStringPart::Expr(Box::new(self.infer_expr(e)))
                        }
                        aelys_syntax::FmtStringPart::Placeholder => TypedFmtStringPart::Placeholder,
                    })
                    .collect();
                (TypedExprKind::FmtString(typed_parts), InferType::String)
            }
            ExprKind::Null => (TypedExprKind::Null, InferType::Null),
            ExprKind::Identifier(name) => self.infer_identifier_expr(name, expr.span),
            ExprKind::Binary { left, op, right } => {
                let typed_left = self.infer_expr(left);
                let typed_right = self.infer_expr(right);
                let result_type = self.infer_binary_op(*op, &typed_left, &typed_right, expr.span);

                (
                    TypedExprKind::Binary {
                        left: Box::new(typed_left),
                        op: *op,
                        right: Box::new(typed_right),
                    },
                    result_type,
                )
            }
            ExprKind::Unary { op, operand } => {
                let typed_operand = self.infer_expr(operand);
                let result_type = self.infer_unary_op(*op, &typed_operand, expr.span);

                (
                    TypedExprKind::Unary {
                        op: *op,
                        operand: Box::new(typed_operand),
                    },
                    result_type,
                )
            }
            ExprKind::And { left, right } => self.infer_logical_expr("and", left, right, expr),
            ExprKind::Or { left, right } => self.infer_logical_expr("or", left, right, expr),
            ExprKind::Call { callee, args } => self.infer_call_expr(callee, args, expr.span),
            ExprKind::Assign { name, value } => self.infer_assign_expr(name, value, expr.span),
            ExprKind::Grouping(inner) => {
                let typed_inner = self.infer_expr(inner);
                let ty = typed_inner.ty.clone();
                (TypedExprKind::Grouping(Box::new(typed_inner)), ty)
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.infer_if_expr(condition, then_branch, else_branch, expr.span),
            ExprKind::Lambda {
                params,
                return_type,
                body,
            } => self.infer_lambda_expr(params, return_type.as_ref(), body, expr.span),
            ExprKind::Member { object, member } => {
                self.infer_member_expr(object, member, expr.span)
            }
            ExprKind::ArrayLiteral {
                element_type,
                elements,
            } => self.infer_array_literal(element_type, elements, expr.span),
            ExprKind::ArraySized { element_type, size } => {
                self.infer_array_sized(element_type, size, expr.span)
            }
            ExprKind::VecLiteral {
                element_type,
                elements,
            } => self.infer_vec_literal(element_type, elements, expr.span),
            ExprKind::Index { object, index } => self.infer_index_expr(object, index, expr.span),
            ExprKind::IndexAssign {
                object,
                index,
                value,
            } => self.infer_index_assign_expr(object, index, value, expr.span),
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => self.infer_range_expr(start, end, *inclusive, expr.span),
            ExprKind::Slice { object, range } => self.infer_slice_expr(object, range, expr.span),
            ExprKind::StructLiteral { name, fields } => {
                self.infer_struct_literal(name, fields, expr.span)
            }
            ExprKind::Cast { expr: inner, target } => {
                let typed_inner = self.infer_expr(inner);
                let target_ty = InferType::from_annotation(target);
                let src = &typed_inner.ty;
                let allowed = (src.is_numeric() || *src == InferType::Bool || *src == InferType::Dynamic)
                    && (target_ty.is_numeric() || target_ty == InferType::Bool);
                if !allowed {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: target_ty.clone(),
                            found: src.clone(),
                        },
                        span: inner.span,
                        reason: ConstraintReason::InvalidCast,
                    });
                }
                (
                    TypedExprKind::Cast {
                        expr: Box::new(typed_inner),
                        target: target_ty.clone(),
                    },
                    target_ty,
                )
            }
        };

        self.depth -= 1;
        TypedExpr {
            kind,
            ty,
            span: expr.span,
        }
    }

    fn infer_logical_expr(
        &mut self,
        op_label: &str,
        left: &Expr,
        right: &Expr,
        _expr: &Expr,
    ) -> (TypedExprKind, InferType) {
        let typed_left = self.infer_expr(left);
        let typed_right = self.infer_expr(right);

        self.constraints.push(Constraint::equal(
            typed_left.ty.clone(),
            InferType::Bool,
            left.span,
            ConstraintReason::BinaryOp {
                op: op_label.to_string(),
            },
        ));
        self.constraints.push(Constraint::equal(
            typed_right.ty.clone(),
            InferType::Bool,
            right.span,
            ConstraintReason::BinaryOp {
                op: op_label.to_string(),
            },
        ));

        (
            if op_label == "and" {
                TypedExprKind::And {
                    left: Box::new(typed_left),
                    right: Box::new(typed_right),
                }
            } else {
                TypedExprKind::Or {
                    left: Box::new(typed_left),
                    right: Box::new(typed_right),
                }
            },
            InferType::Bool,
        )
    }
}

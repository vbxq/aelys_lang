use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::TypedExpr;
use crate::types::InferType;
use aelys_common::{Warning, WarningKind};
use aelys_syntax::{BinaryOp, Span, UnaryOp};

impl TypeInference {
    pub(super) fn infer_binary_op(
        &mut self,
        op: BinaryOp,
        left: &TypedExpr,
        right: &TypedExpr,
        span: Span,
    ) -> InferType {
        match op {
            BinaryOp::Add | BinaryOp::Sub | BinaryOp::Mul | BinaryOp::Div => {
                let result_type = self.type_gen.fresh();

                self.constraints.push(Constraint::equal(
                    left.ty.clone(),
                    right.ty.clone(),
                    span,
                    ConstraintReason::BinaryOp { op: op.to_string() },
                ));

                self.constraints.push(Constraint::equal(
                    left.ty.clone(),
                    result_type.clone(),
                    span,
                    ConstraintReason::BinaryOp { op: op.to_string() },
                ));

                if op == BinaryOp::Add {
                    let mut options = InferType::all_numeric_types();
                    options.push(InferType::String);
                    self.constraints.push(Constraint::one_of(
                        left.ty.clone(),
                        options,
                        span,
                        ConstraintReason::BinaryOp { op: op.to_string() },
                    ));
                } else {
                    self.constraints.push(Constraint::one_of(
                        left.ty.clone(),
                        InferType::all_numeric_types(),
                        span,
                        ConstraintReason::BinaryOp { op: op.to_string() },
                    ));
                }

                result_type
            }

            BinaryOp::Mod => {
                let result_type = self.type_gen.fresh();

                self.constraints.push(Constraint::equal(
                    left.ty.clone(),
                    right.ty.clone(),
                    span,
                    ConstraintReason::BinaryOp { op: op.to_string() },
                ));

                self.constraints.push(Constraint::equal(
                    left.ty.clone(),
                    result_type.clone(),
                    span,
                    ConstraintReason::BinaryOp { op: op.to_string() },
                ));

                self.constraints.push(Constraint::one_of(
                    left.ty.clone(),
                    InferType::all_numeric_types(),
                    span,
                    ConstraintReason::BinaryOp { op: op.to_string() },
                ));

                result_type
            }

            BinaryOp::Lt | BinaryOp::Le | BinaryOp::Gt | BinaryOp::Ge => {
                self.constraints.push(Constraint::equal(
                    left.ty.clone(),
                    right.ty.clone(),
                    span,
                    ConstraintReason::Comparison,
                ));

                self.constraints.push(Constraint::one_of(
                    left.ty.clone(),
                    InferType::all_numeric_types(),
                    span,
                    ConstraintReason::Comparison,
                ));

                InferType::Bool
            }

            BinaryOp::Eq | BinaryOp::Ne => {
                if left.ty.is_concrete() && right.ty.is_concrete() && left.ty != right.ty {
                    self.warnings.push(Warning::new(
                        WarningKind::IncompatibleComparison {
                            left: left.ty.to_string(),
                            right: right.ty.to_string(),
                            op: op.to_string(),
                        },
                        span,
                    ));
                } else {
                    self.constraints.push(Constraint::equal(
                        left.ty.clone(),
                        right.ty.clone(),
                        span,
                        ConstraintReason::Comparison,
                    ));
                }

                InferType::Bool
            }

            BinaryOp::Shl
            | BinaryOp::Shr
            | BinaryOp::BitAnd
            | BinaryOp::BitOr
            | BinaryOp::BitXor => {
                self.constraints.push(Constraint::equal(
                    left.ty.clone(),
                    right.ty.clone(),
                    span,
                    ConstraintReason::BitwiseOp { op: op.to_string() },
                ));

                self.constraints.push(Constraint::one_of(
                    left.ty.clone(),
                    InferType::all_integer_types(),
                    span,
                    ConstraintReason::BitwiseOp { op: op.to_string() },
                ));

                let result_type = self.type_gen.fresh();
                self.constraints.push(Constraint::equal(
                    left.ty.clone(),
                    result_type.clone(),
                    span,
                    ConstraintReason::BitwiseOp { op: op.to_string() },
                ));

                result_type
            }
        }
    }

    pub(super) fn infer_unary_op(
        &mut self,
        op: UnaryOp,
        operand: &TypedExpr,
        span: Span,
    ) -> InferType {
        match op {
            UnaryOp::Neg => {
                self.constraints.push(Constraint::one_of(
                    operand.ty.clone(),
                    InferType::all_numeric_types(),
                    span,
                    ConstraintReason::BinaryOp {
                        op: "-".to_string(),
                    },
                ));
                operand.ty.clone()
            }
            UnaryOp::Not => InferType::Bool,
            UnaryOp::BitNot => {
                self.constraints.push(Constraint::one_of(
                    operand.ty.clone(),
                    InferType::all_integer_types(),
                    span,
                    ConstraintReason::BitwiseOp {
                        op: "~".to_string(),
                    },
                ));
                operand.ty.clone()
            }
        }
    }
}

use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError};
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
                // when both operands have the same concrete type, the result type is already determined, this means no type variable needed.
                // ysing a concrete type here lets downstream narrowing (for example in return position) see the real type instead of an opaque Var
                let result_type = if left.ty == right.ty && left.ty.is_concrete() {
                    left.ty.clone()
                } else {
                    let fresh = self.type_gen.fresh();

                    self.constraints.push(Constraint::equal(
                        left.ty.clone(),
                        right.ty.clone(),
                        span,
                        ConstraintReason::BinaryOp { op: op.to_string() },
                    ));

                    self.constraints.push(Constraint::equal(
                        left.ty.clone(),
                        fresh.clone(),
                        span,
                        ConstraintReason::BinaryOp { op: op.to_string() },
                    ));

                    fresh
                };

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
                let result_type = if left.ty == right.ty && left.ty.is_concrete() {
                    left.ty.clone()
                } else {
                    let fresh = self.type_gen.fresh();

                    self.constraints.push(Constraint::equal(
                        left.ty.clone(),
                        right.ty.clone(),
                        span,
                        ConstraintReason::BinaryOp { op: op.to_string() },
                    ));

                    self.constraints.push(Constraint::equal(
                        left.ty.clone(),
                        fresh.clone(),
                        span,
                        ConstraintReason::BinaryOp { op: op.to_string() },
                    ));

                    fresh
                };

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
                // Reject comparison on data enums (enums with payload fields).
                // Simple enums (all unit variants) are fine — they're just i32 tags.
                // Also reject comparison on structs — no codegen support for deep equality.
                for operand_ty in [&left.ty, &right.ty] {
                    if let InferType::Enum(name, _) = operand_ty {
                        if let Some(def) = self.type_table.get_enum(name) {
                            if def.variants.iter().any(|v| !v.data.is_empty()) {
                                self.errors.push(TypeError::member_access(
                                    format!(
                                        "comparison (`{}`) is not supported for enum `{}` \
                                         because it has data variants; use `match` instead",
                                        op, name
                                    ),
                                    span,
                                ));
                                return InferType::Bool;
                            }
                        }
                    }
                    if let InferType::Struct(name) = operand_ty {
                        self.errors.push(TypeError::member_access(
                            format!(
                                "comparison (`{}`) is not supported for struct `{}`",
                                op, name
                            ),
                            span,
                        ));
                        return InferType::Bool;
                    }
                }

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
                let result_type = if left.ty == right.ty && left.ty.is_concrete() {
                    left.ty.clone()
                } else {
                    self.constraints.push(Constraint::equal(
                        left.ty.clone(),
                        right.ty.clone(),
                        span,
                        ConstraintReason::BitwiseOp { op: op.to_string() },
                    ));

                    let fresh = self.type_gen.fresh();
                    self.constraints.push(Constraint::equal(
                        left.ty.clone(),
                        fresh.clone(),
                        span,
                        ConstraintReason::BitwiseOp { op: op.to_string() },
                    ));

                    fresh
                };

                self.constraints.push(Constraint::one_of(
                    left.ty.clone(),
                    InferType::all_integer_types(),
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
            UnaryOp::Not => {
                self.constraints.push(Constraint::equal(
                    operand.ty.clone(),
                    InferType::Bool,
                    span,
                    ConstraintReason::BinaryOp {
                        op: "!".to_string(),
                    },
                ));
                InferType::Bool
            }
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

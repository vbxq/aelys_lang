use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{BinaryOp, Span, UnaryOp};

pub(super) enum Fold {
    NotConst,
    Reported,
}

fn lit(v: i128, ty: &InferType) -> String {
    format!("{v}_{ty}")
}

fn min_name(v: i128, ty: &InferType) -> String {
    match ty.int_bounds() {
        Some((lo, _)) if v == lo && lo < 0 => format!("{ty}::MIN"),
        _ => lit(v, ty),
    }
}

pub(crate) fn negate_label(v: i128, ty: &InferType) -> String {
    format!("attempt to negate `{}`, which would overflow", min_name(v, ty))
}

pub(crate) fn compute_label(l: i128, sym: &str, r: i128, ty: &InferType) -> String {
    format!(
        "attempt to compute `{} {sym} {}`, which would overflow",
        min_name(l, ty),
        lit(r, ty)
    )
}

pub(crate) fn shift_label(op: BinaryOp, r: i128, ty: &InferType) -> String {
    let dir = if op == BinaryOp::Shl { "left" } else { "right" };
    format!("attempt to shift {dir} by `{}`, which would overflow", lit(r, ty))
}

pub(crate) fn div_zero_label(op: BinaryOp, l: i128, ty: &InferType) -> String {
    if op == BinaryOp::Div {
        format!("attempt to divide `{}` by zero", lit(l, ty))
    } else {
        format!(
            "attempt to calculate the remainder of `{}` with a divisor of zero",
            lit(l, ty)
        )
    }
}

impl TypeInference {
    pub(super) fn fold_int_in(&mut self, e: &TypedExpr, ty: &InferType) -> Result<i128, Fold> {
        let Some((lo, hi)) = ty.int_bounds() else {
            return Err(Fold::NotConst);
        };
        match &e.kind {
            TypedExprKind::Int(v) => self.leaf_in(*v as i128, e, ty),
            TypedExprKind::Grouping(inner) => self.fold_int_in(inner, ty),
            TypedExprKind::Unary {
                op: UnaryOp::Neg,
                operand,
            } => {
                if let TypedExprKind::Int(v) = &operand.kind {
                    return self.leaf_in(-(*v as i128), e, ty);
                }
                let v = self.fold_int_in(operand, ty)?;
                if -v < lo || -v > hi {
                    return Err(self.overflow(negate_label(v, ty), e.span));
                }
                Ok(-v)
            }
            TypedExprKind::Unary {
                op: UnaryOp::BitNot,
                operand,
            } => {
                let v = self.fold_int_in(operand, ty)?;
                Ok(if lo < 0 { !v } else { hi ^ v })
            }
            TypedExprKind::Binary { left, op, right } => {
                if !matches!(
                    op,
                    BinaryOp::Add
                        | BinaryOp::Sub
                        | BinaryOp::Mul
                        | BinaryOp::Div
                        | BinaryOp::Mod
                        | BinaryOp::BitAnd
                        | BinaryOp::BitOr
                        | BinaryOp::BitXor
                        | BinaryOp::Shl
                        | BinaryOp::Shr
                ) {
                    return Err(Fold::NotConst);
                }
                let l = self.fold_int_in(left, ty)?;
                let r = self.fold_int_in(right, ty)?;
                self.fold_binary(*op, l, r, e, ty, (lo, hi))
            }
            _ => Err(Fold::NotConst),
        }
    }

    fn fold_binary(
        &mut self,
        op: BinaryOp,
        l: i128,
        r: i128,
        e: &TypedExpr,
        ty: &InferType,
        (lo, hi): (i128, i128),
    ) -> Result<i128, Fold> {
        let (v, sym) = match op {
            BinaryOp::Add => (l + r, "+"),
            BinaryOp::Sub => (l - r, "-"),
            BinaryOp::Mul => (l * r, "*"),
            BinaryOp::Div | BinaryOp::Mod if r == 0 => {
                self.errors.push(TypeError::coded(
                    "E0451",
                    "this operation will panic at runtime",
                    &div_zero_label(op, l, ty),
                    e.span,
                ));
                return Err(Fold::Reported);
            }
            BinaryOp::Div => (l / r, "/"),
            BinaryOp::Mod if l == lo && r == -1 && lo < 0 => (hi + 1, "%"),
            BinaryOp::Mod => (l % r, "%"),
            BinaryOp::BitAnd => return Ok(l & r),
            BinaryOp::BitOr => return Ok(l | r),
            BinaryOp::BitXor => return Ok(l ^ r),
            BinaryOp::Shl | BinaryOp::Shr => {
                let bits = 128 - (hi - lo).leading_zeros() as i128;
                if r < 0 || r >= bits {
                    return Err(self.overflow(shift_label(op, r, ty), e.span));
                }
                if op == BinaryOp::Shr {
                    return Ok(l >> r);
                }
                let wide = (l << r) & ((1i128 << bits) - 1);
                return Ok(if wide > hi {
                    wide - (1i128 << bits)
                } else {
                    wide
                });
            }
            _ => return Err(Fold::NotConst),
        };
        if v < lo || v > hi {
            return Err(self.overflow(compute_label(l, sym, r, ty), e.span));
        }
        Ok(v)
    }

    fn leaf_in(&mut self, v: i128, e: &TypedExpr, ty: &InferType) -> Result<i128, Fold> {
        if ty.int_bounds().is_some_and(|(lo, hi)| v >= lo && v <= hi) {
            return Ok(v);
        }
        self.errors.push(TypeError {
            kind: TypeErrorKind::Mismatch {
                expected: ty.clone(),
                found: InferType::I64,
            },
            span: e.span,
            reason: ConstraintReason::IntLiteralOverflow {
                value: v as i64,
                target: ty.clone(),
            },
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        });
        Err(Fold::Reported)
    }

    pub(super) fn reject_const_shift_overflow(
        &mut self,
        op: BinaryOp,
        amount: &TypedExpr,
        width: &InferType,
        span: Span,
    ) {
        let (Some((lo, hi)), Some(r)) = (width.int_bounds(), Self::closed_const_int(amount)) else {
            return;
        };
        let r = r as i128;
        if r < 0 || r >= 128 - (hi - lo).leading_zeros() as i128 {
            self.overflow(shift_label(op, r, width), span);
        }
    }

    fn overflow(&mut self, label: String, span: Span) -> Fold {
        self.errors.push(TypeError::coded(
            "E0450",
            "this arithmetic operation will overflow",
            &label,
            span,
        ));
        Fold::Reported
    }
}

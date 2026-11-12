use super::super::super::ConstantFolder;
use aelys_sema::{InferType, TypedExpr, TypedExprKind};
use aelys_syntax::BinaryOp;

impl ConstantFolder {
    pub(super) fn fold_int_binary(
        &mut self,
        a: i64,
        op: BinaryOp,
        b: i64,
        operand_ty: &InferType,
        original: &TypedExpr,
    ) -> Option<TypedExpr> {
        let bool_result = |this: &mut Self, v: bool| {
            this.stats.constants_folded += 1;
            Some(TypedExpr::new(
                TypedExprKind::Bool(v),
                InferType::Bool,
                original.span,
            ))
        };
        let result_ty = if original.ty.is_integer() {
            original.ty.clone()
        } else {
            operand_ty.clone()
        };
        let int_result = |this: &mut Self, v: i64| {
            this.stats.constants_folded += 1;
            Some(TypedExpr::new(
                TypedExprKind::Int(super::super::super::truncate_to_type(v, &result_ty)),
                result_ty.clone(),
                original.span,
            ))
        };

        let width = bit_width(operand_ty)?;
        let unsigned = is_unsigned(operand_ty);
        let a = super::super::super::truncate_to_type(a, operand_ty);
        let b_full = b;
        let b = super::super::super::truncate_to_type(b, operand_ty);
        let ordered = |x: i64, y: i64| {
            if unsigned {
                (x as u64).cmp(&(y as u64))
            } else {
                x.cmp(&y)
            }
        };

        match op {
            BinaryOp::Lt => return bool_result(self, ordered(a, b).is_lt()),
            BinaryOp::Le => return bool_result(self, ordered(a, b).is_le()),
            BinaryOp::Gt => return bool_result(self, ordered(a, b).is_gt()),
            BinaryOp::Ge => return bool_result(self, ordered(a, b).is_ge()),
            BinaryOp::Eq => return bool_result(self, a == b),
            BinaryOp::Ne => return bool_result(self, a != b),
            BinaryOp::BitAnd => return int_result(self, a & b),
            BinaryOp::BitOr => return int_result(self, a | b),
            BinaryOp::BitXor => return int_result(self, a ^ b),
            // the runtime masks the amount by the width of the shifted value
            BinaryOp::Shl => {
                let amount = (b_full as u64 & (width as u64 - 1)) as u32;
                return int_result(self, a.wrapping_shl(amount));
            }
            BinaryOp::Shr => {
                let amount = (b_full as u64 & (width as u64 - 1)) as u32;
                let shifted = if unsigned {
                    ((a as u64) >> amount) as i64
                } else {
                    a >> amount
                };
                return int_result(self, shifted);
            }
            _ => {}
        }

        let result = match op {
            BinaryOp::Add => a.wrapping_add(b),
            BinaryOp::Sub => a.wrapping_sub(b),
            BinaryOp::Mul => a.wrapping_mul(b),
            BinaryOp::Div | BinaryOp::Mod => {
                if b == 0 || (!unsigned && b == -1 && a == signed_min(width)) {
                    return None;
                }
                match (op, unsigned) {
                    (BinaryOp::Div, true) => ((a as u64) / (b as u64)) as i64,
                    (BinaryOp::Div, false) => a / b,
                    (_, true) => ((a as u64) % (b as u64)) as i64,
                    (_, false) => a % b,
                }
            }
            _ => return None,
        };
        int_result(self, result)
    }
}

fn bit_width(ty: &InferType) -> Option<u32> {
    match ty {
        InferType::I8 | InferType::U8 => Some(8),
        InferType::I16 | InferType::U16 => Some(16),
        InferType::I32 | InferType::U32 => Some(32),
        InferType::I64 | InferType::U64 => Some(64),
        _ => None,
    }
}

fn is_unsigned(ty: &InferType) -> bool {
    matches!(
        ty,
        InferType::U8 | InferType::U16 | InferType::U32 | InferType::U64
    )
}

fn signed_min(width: u32) -> i64 {
    i64::MIN >> (64 - width)
}

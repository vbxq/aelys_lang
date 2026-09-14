use super::super::super::ConstantFolder;
use aelys_sema::{InferType, TypedExpr, TypedExprKind};
use aelys_syntax::BinaryOp;

impl ConstantFolder {
    pub(super) fn fold_int_binary(
        &mut self,
        a: i64,
        op: BinaryOp,
        b: i64,
        original: &TypedExpr,
    ) -> Option<TypedExpr> {
        // helper to emit a bool result
        let bool_result = |this: &mut Self, v: bool| {
            this.stats.constants_folded += 1;
            Some(TypedExpr::new(
                TypedExprKind::Bool(v),
                InferType::Bool,
                original.span,
            ))
        };
        // helper to emit an int result
        let result_ty = if original.ty.is_integer() {
            original.ty.clone()
        } else {
            InferType::I64
        };
        let int_result = |this: &mut Self, v: i64| {
            this.stats.constants_folded += 1;
            Some(TypedExpr::new(
                TypedExprKind::Int(v),
                result_ty.clone(),
                original.span,
            ))
        };

        match op {
            // comparisons -> bool
            BinaryOp::Lt => return bool_result(self, a < b),
            BinaryOp::Le => return bool_result(self, a <= b),
            BinaryOp::Gt => return bool_result(self, a > b),
            BinaryOp::Ge => return bool_result(self, a >= b),
            BinaryOp::Eq => return bool_result(self, a == b),
            BinaryOp::Ne => return bool_result(self, a != b),
            // bitwise - truncate to model sign extension correctly for narrow types
            BinaryOp::BitAnd => {
                return int_result(
                    self,
                    super::super::super::truncate_to_type(a & b, &result_ty),
                );
            }
            BinaryOp::BitOr => {
                return int_result(
                    self,
                    super::super::super::truncate_to_type(a | b, &result_ty),
                );
            }
            BinaryOp::BitXor => {
                return int_result(
                    self,
                    super::super::super::truncate_to_type(a ^ b, &result_ty),
                );
            }
            // shifts need bounds checking
            BinaryOp::Shl => {
                if !(0..=63).contains(&b) {
                    return None;
                }
                let result = a.wrapping_shl(b as u32);
                let result = super::super::super::truncate_to_type(result, &result_ty);
                return int_result(self, result);
            }
            BinaryOp::Shr => {
                if !(0..=63).contains(&b) {
                    return None;
                }
                let result = super::super::super::truncate_to_type(a >> (b as u32), &result_ty);
                return int_result(self, result);
            }
            _ => {}
        }

        // arithmetic - use wrapping for Add/Sub/Mul so narrower types wrap
        // correctly. Div/Mod can't overflow so keep checked for divide-by-zero.
        let result = match op {
            BinaryOp::Add => a.wrapping_add(b),
            BinaryOp::Sub => a.wrapping_sub(b),
            BinaryOp::Mul => a.wrapping_mul(b),
            BinaryOp::Div if b != 0 => a.checked_div(b)?,
            BinaryOp::Mod if b != 0 => a.checked_rem(b)?,
            _ => return None,
        };
        // Truncate to the actual target type so that narrower-type wrapping is
        // correctly modelled. e.g. 100_i8 + 100_i8 folds to -56, not 200.
        let result = super::super::super::truncate_to_type(result, &result_ty);
        int_result(self, result)
    }
}

use super::super::super::ConstantFolder;
use aelys_sema::{InferType, TypedExpr, TypedExprKind};
use aelys_syntax::BinaryOp;

impl ConstantFolder {
    pub(super) fn fold_int_binary(&mut self, a: i64, op: BinaryOp, b: i64, original: &TypedExpr) -> Option<TypedExpr> {
        use super::super::super::is_in_vm_range;
        if !is_in_vm_range(a) || !is_in_vm_range(b) { return None; }

        // helper to emit a bool result
        let bool_result = |this: &mut Self, v: bool| {
            this.stats.constants_folded += 1;
            Some(TypedExpr::new(TypedExprKind::Bool(v), InferType::Bool, original.span))
        };
        // helper to emit an int result
        let int_result = |this: &mut Self, v: i64| {
            this.stats.constants_folded += 1;
            Some(TypedExpr::new(TypedExprKind::Int(v), InferType::Int, original.span))
        };

        match op {
            // comparisons -> bool
            BinaryOp::Lt => return bool_result(self, a < b),
            BinaryOp::Le => return bool_result(self, a <= b),
            BinaryOp::Gt => return bool_result(self, a > b),
            BinaryOp::Ge => return bool_result(self, a >= b),
            BinaryOp::Eq => return bool_result(self, a == b),
            BinaryOp::Ne => return bool_result(self, a != b),
            // bitwise - no overflow possible
            BinaryOp::BitAnd => return int_result(self, a & b),
            BinaryOp::BitOr => return int_result(self, a | b),
            BinaryOp::BitXor => return int_result(self, a ^ b),
            // shifts need bounds checking
            BinaryOp::Shl => {
                if b < 0 || b > 63 { return None; }
                let result = a.checked_shl(b as u32)?;
                if !is_in_vm_range(result) { return None; }
                return int_result(self, result);
            }
            BinaryOp::Shr => {
                if b < 0 || b > 63 { return None; }
                return int_result(self, a >> (b as u32));
            }
            _ => {}
        }

        // arithmetic - check for overflow
        let result = match op {
            BinaryOp::Add => a.checked_add(b)?,
            BinaryOp::Sub => a.checked_sub(b)?,
            BinaryOp::Mul => a.checked_mul(b)?,
            BinaryOp::Div if b != 0 => a.checked_div(b)?,
            BinaryOp::Mod if b != 0 => a.checked_rem(b)?,
            _ => return None,
        };
        if !is_in_vm_range(result) { return None; }
        int_result(self, result)
    }
}

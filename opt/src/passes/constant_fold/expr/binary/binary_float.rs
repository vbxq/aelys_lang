use super::super::super::ConstantFolder;
use aelys_sema::{InferType, TypedExpr, TypedExprKind};
use aelys_syntax::BinaryOp;

impl ConstantFolder {
    pub(super) fn fold_float_binary(
        &mut self,
        a: f64,
        op: BinaryOp,
        b: f64,
        float_ty: &InferType,
        original: &TypedExpr,
    ) -> Option<TypedExpr> {
        let narrow = *float_ty == InferType::F32;
        // the runtime holds an f32 operand already rounded, so folding the wider literal answers for a number the program never has
        let (a, b) = if narrow {
            (a as f32 as f64, b as f32 as f64)
        } else {
            (a, b)
        };

        let bool_result = |this: &mut Self, v: bool| {
            this.stats.constants_folded += 1;
            Some(TypedExpr::new(
                TypedExprKind::Bool(v),
                InferType::Bool,
                original.span,
            ))
        };

        match op {
            BinaryOp::Lt => return bool_result(self, a < b),
            BinaryOp::Le => return bool_result(self, a <= b),
            BinaryOp::Gt => return bool_result(self, a > b),
            BinaryOp::Ge => return bool_result(self, a >= b),
            BinaryOp::Eq => return bool_result(self, a == b),
            BinaryOp::Ne => return bool_result(self, a != b),
            _ => {}
        }

        let result = match op {
            BinaryOp::Add => a + b,
            BinaryOp::Sub => a - b,
            BinaryOp::Mul => a * b,
            BinaryOp::Div if b != 0.0 => a / b,
            BinaryOp::Mod if b != 0.0 => a % b,
            _ => return None,
        };

        let result = if narrow { result as f32 as f64 } else { result };

        // don't fold to inf/nan - let runtime handle it
        if result.is_nan() || result.is_infinite() {
            return None;
        }

        self.stats.constants_folded += 1;
        Some(TypedExpr::new(
            TypedExprKind::Float(result),
            float_ty.clone(),
            original.span,
        ))
    }
}

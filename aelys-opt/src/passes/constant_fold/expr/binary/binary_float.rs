use super::super::super::ConstantFolder;
use aelys_sema::{InferType, TypedExpr, TypedExprKind};
use aelys_syntax::BinaryOp;

impl ConstantFolder {
    pub(super) fn fold_float_binary(
        &mut self,
        a: f64,
        op: BinaryOp,
        b: f64,
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

        // comparisons first
        match op {
            BinaryOp::Lt => return bool_result(self, a < b),
            BinaryOp::Le => return bool_result(self, a <= b),
            BinaryOp::Gt => return bool_result(self, a > b),
            BinaryOp::Ge => return bool_result(self, a >= b),
            BinaryOp::Eq => return bool_result(self, a == b),
            BinaryOp::Ne => return bool_result(self, a != b),
            _ => {}
        }

        // arithmetic
        let result = match op {
            BinaryOp::Add => a + b,
            BinaryOp::Sub => a - b,
            BinaryOp::Mul => a * b,
            BinaryOp::Div if b != 0.0 => a / b,
            BinaryOp::Mod if b != 0.0 => a % b,
            _ => return None,
        };

        // don't fold to inf/nan - let runtime handle it
        if result.is_nan() || result.is_infinite() {
            return None;
        }

        self.stats.constants_folded += 1;
        Some(TypedExpr::new(
            TypedExprKind::Float(result),
            InferType::Float,
            original.span,
        ))
    }
}

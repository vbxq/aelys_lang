use super::super::ConstantFolder;
use aelys_sema::{InferType, TypedExpr, TypedExprKind};
use aelys_syntax::UnaryOp;

impl ConstantFolder {
    pub(super) fn try_fold_unary(
        &mut self,
        op: UnaryOp,
        operand: &TypedExpr,
        original: &TypedExpr,
    ) -> Option<TypedExpr> {
        let operand_folded = self.try_fold(operand);
        let operand_val = operand_folded.as_ref().unwrap_or(operand);

        match (&operand_val.kind, op) {
            (TypedExprKind::Int(n), UnaryOp::Neg) => {
                if !super::super::is_in_vm_range(*n) {
                    return None;
                }
                let result = n.checked_neg()?;
                if !super::super::is_in_vm_range(result) {
                    return None;
                }
                self.stats.constants_folded += 1;
                Some(TypedExpr::new(
                    TypedExprKind::Int(result),
                    InferType::I64,
                    original.span,
                ))
            }
            (TypedExprKind::Float(f), UnaryOp::Neg) => {
                self.stats.constants_folded += 1;
                Some(TypedExpr::new(
                    TypedExprKind::Float(-f),
                    InferType::F64,
                    original.span,
                ))
            }
            (TypedExprKind::Bool(b), UnaryOp::Not) => {
                self.stats.constants_folded += 1;
                Some(TypedExpr::new(
                    TypedExprKind::Bool(!b),
                    InferType::Bool,
                    original.span,
                ))
            }
            (TypedExprKind::Int(n), UnaryOp::BitNot) => {
                if !super::super::is_in_vm_range(*n) {
                    return None;
                }
                self.stats.constants_folded += 1;
                Some(TypedExpr::new(
                    TypedExprKind::Int(!*n),
                    InferType::I64,
                    original.span,
                ))
            }
            _ => {
                // couldn't fold fully, but propagate partial result
                operand_folded.map(|folded| {
                    TypedExpr::new(
                        TypedExprKind::Unary {
                            op,
                            operand: Box::new(folded),
                        },
                        original.ty.clone(),
                        original.span,
                    )
                })
            }
        }
    }
}

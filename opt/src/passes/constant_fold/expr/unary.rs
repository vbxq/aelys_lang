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
                let result = n.wrapping_neg();
                let result_ty = if original.ty.is_integer() {
                    original.ty.clone()
                } else {
                    operand_val.ty.clone()
                };
                let result = super::super::truncate_to_type(result, &result_ty);
                if !super::super::is_in_vm_range(result) {
                    return None;
                }
                self.stats.constants_folded += 1;
                Some(TypedExpr::new(
                    TypedExprKind::Int(result),
                    result_ty,
                    original.span,
                ))
            }
            (TypedExprKind::Float(f), UnaryOp::Neg) => {
                self.stats.constants_folded += 1;
                Some(TypedExpr::new(
                    TypedExprKind::Float(-f),
                    original.ty.clone(),
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
                let result_ty = if original.ty.is_integer() {
                    original.ty.clone()
                } else {
                    operand_val.ty.clone()
                };
                let result = super::super::truncate_to_type(!*n, &result_ty);
                self.stats.constants_folded += 1;
                Some(TypedExpr::new(
                    TypedExprKind::Int(result),
                    result_ty,
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

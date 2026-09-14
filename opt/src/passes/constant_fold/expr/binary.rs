use super::super::ConstantFolder;
use aelys_sema::{InferType, TypedExpr, TypedExprKind};
use aelys_syntax::BinaryOp;

mod binary_float;
mod binary_int;

impl ConstantFolder {
    pub(super) fn try_fold_binary(
        &mut self,
        left: &TypedExpr,
        op: BinaryOp,
        right: &TypedExpr,
        original: &TypedExpr,
    ) -> Option<TypedExpr> {
        let left_folded = self.try_fold(left);
        let right_folded = self.try_fold(right);
        let left_val = left_folded.as_ref().unwrap_or(left);
        let right_val = right_folded.as_ref().unwrap_or(right);
        let left_unwrapped = Self::unwrap_grouping(left_val);
        let right_unwrapped = Self::unwrap_grouping(right_val);

        match (&left_unwrapped.kind, &right_unwrapped.kind) {
            (TypedExprKind::Int(a), TypedExprKind::Int(b)) => {
                self.fold_int_binary(*a, op, *b, original)
            }
            (TypedExprKind::Float(a), TypedExprKind::Float(b)) => {
                let float_ty = float_format(&left_unwrapped.ty, &right_unwrapped.ty, &original.ty);
                self.fold_float_binary(*a, op, *b, &float_ty, original)
            }
            (TypedExprKind::Int(a), TypedExprKind::Float(b)) => {
                let float_ty = float_format(&left_unwrapped.ty, &right_unwrapped.ty, &original.ty);
                self.fold_float_binary(*a as f64, op, *b, &float_ty, original)
            }
            (TypedExprKind::Float(a), TypedExprKind::Int(b)) => {
                let float_ty = float_format(&left_unwrapped.ty, &right_unwrapped.ty, &original.ty);
                self.fold_float_binary(*a, op, *b as f64, &float_ty, original)
            }
            (TypedExprKind::String(a), TypedExprKind::String(b)) if op == BinaryOp::Add => {
                self.fold_string_concat(a, b, original)
            }
            (TypedExprKind::Bool(a), TypedExprKind::Bool(b)) => {
                self.fold_bool_comparison(*a, op, *b, original)
            }
            _ => {
                // can't fully fold, but propagate any partial folding we did
                if left_folded.is_some() || right_folded.is_some() {
                    Some(TypedExpr::new(
                        TypedExprKind::Binary {
                            left: Box::new(left_folded.unwrap_or_else(|| left.clone())),
                            op,
                            right: Box::new(right_folded.unwrap_or_else(|| right.clone())),
                        },
                        original.ty.clone(),
                        original.span,
                    ))
                } else {
                    None
                }
            }
        }
    }

    pub(super) fn fold_string_concat(
        &mut self,
        a: &str,
        b: &str,
        original: &TypedExpr,
    ) -> Option<TypedExpr> {
        if a.len() + b.len() > super::super::MAX_FOLDED_STRING_LEN {
            return None;
        }
        self.stats.constants_folded += 1;
        Some(TypedExpr::new(
            TypedExprKind::String(format!("{}{}", a, b)),
            InferType::String,
            original.span,
        ))
    }

    pub(super) fn fold_bool_comparison(
        &mut self,
        a: bool,
        op: BinaryOp,
        b: bool,
        original: &TypedExpr,
    ) -> Option<TypedExpr> {
        let result = match op {
            BinaryOp::Eq => a == b,
            BinaryOp::Ne => a != b,
            _ => return None,
        };
        self.stats.constants_folded += 1;
        Some(TypedExpr::new(
            TypedExprKind::Bool(result),
            InferType::Bool,
            original.span,
        ))
    }
}

fn float_format(left: &InferType, right: &InferType, original: &InferType) -> InferType {
    if original.is_float() {
        return original.clone();
    }
    let widened = *left == InferType::F64 || *right == InferType::F64;
    let narrow = *left == InferType::F32 || *right == InferType::F32;
    if narrow && !widened {
        InferType::F32
    } else {
        InferType::F64
    }
}

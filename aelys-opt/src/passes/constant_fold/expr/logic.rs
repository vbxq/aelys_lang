use super::super::ConstantFolder;
use aelys_sema::{InferType, TypedExpr, TypedExprKind};

impl ConstantFolder {
    // short-circuit AND: false and X -> false, true and X -> X
    pub(super) fn try_fold_and(
        &mut self,
        left: &TypedExpr,
        right: &TypedExpr,
        original: &TypedExpr,
    ) -> Option<TypedExpr> {
        let left_folded = self.try_fold(left);
        let left_val = left_folded.as_ref().unwrap_or(left);

        match &left_val.kind {
            TypedExprKind::Bool(false) => {
                self.stats.constants_folded += 1;
                return Some(TypedExpr::new(
                    TypedExprKind::Bool(false),
                    InferType::Bool,
                    original.span,
                ));
            }
            TypedExprKind::Bool(true) => {
                self.stats.constants_folded += 1;
                return Some(self.try_fold(right).unwrap_or_else(|| right.clone()));
            }
            _ => {}
        }

        let right_folded = self.try_fold(right);
        if left_folded.is_some() || right_folded.is_some() {
            Some(TypedExpr::new(
                TypedExprKind::And {
                    left: Box::new(left_folded.unwrap_or_else(|| left.clone())),
                    right: Box::new(right_folded.unwrap_or_else(|| right.clone())),
                },
                original.ty.clone(),
                original.span,
            ))
        } else {
            None
        }
    }

    // short-circuit OR: true or X -> true, false or X -> X
    pub(super) fn try_fold_or(
        &mut self,
        left: &TypedExpr,
        right: &TypedExpr,
        original: &TypedExpr,
    ) -> Option<TypedExpr> {
        let left_folded = self.try_fold(left);
        let left_val = left_folded.as_ref().unwrap_or(left);

        match &left_val.kind {
            TypedExprKind::Bool(true) => {
                self.stats.constants_folded += 1;
                return Some(TypedExpr::new(
                    TypedExprKind::Bool(true),
                    InferType::Bool,
                    original.span,
                ));
            }
            TypedExprKind::Bool(false) => {
                self.stats.constants_folded += 1;
                return Some(self.try_fold(right).unwrap_or_else(|| right.clone()));
            }
            _ => {}
        }

        let right_folded = self.try_fold(right);
        if left_folded.is_some() || right_folded.is_some() {
            Some(TypedExpr::new(
                TypedExprKind::Or {
                    left: Box::new(left_folded.unwrap_or_else(|| left.clone())),
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

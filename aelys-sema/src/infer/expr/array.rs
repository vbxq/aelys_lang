use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::{InferType, ResolvedType};
use aelys_syntax::{Expr, Span, TypeAnnotation};

impl TypeInference {
    pub(super) fn infer_array_literal(
        &mut self,
        element_type: &Option<TypeAnnotation>,
        elements: &[Expr],
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_elements: Vec<TypedExpr> = elements.iter().map(|e| self.infer_expr(e)).collect();

        let (elem_ty, resolved_elem) = if let Some(ann) = element_type {
            let ty = self.type_from_annotation(ann);
            let resolved = ResolvedType::from_infer_type(&ty);
            (ty, Some(resolved))
        } else if typed_elements.is_empty() {
            (self.type_gen.fresh(), None)
        } else {
            let first_ty = typed_elements[0].ty.clone();
            for elem in typed_elements.iter().skip(1) {
                self.constraints.push(Constraint::equal(
                    elem.ty.clone(),
                    first_ty.clone(),
                    elem.span,
                    ConstraintReason::ArrayElement,
                ));
            }
            (first_ty, None)
        };

        (
            TypedExprKind::ArrayLiteral {
                element_type: resolved_elem,
                elements: typed_elements,
            },
            InferType::Array(Box::new(elem_ty)),
        )
    }

    pub(super) fn infer_array_sized(
        &mut self,
        element_type: &Option<TypeAnnotation>,
        size: &Expr,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_size = self.infer_expr(size);

        self.constraints.push(Constraint::equal(
            typed_size.ty.clone(),
            InferType::Int,
            span,
            ConstraintReason::ArrayIndex,
        ));

        let (elem_ty, resolved_elem) = if let Some(ann) = element_type {
            let ty = self.type_from_annotation(ann);
            let resolved = ResolvedType::from_infer_type(&ty);
            (ty, Some(resolved))
        } else {
            (InferType::Dynamic, None)
        };

        (
            TypedExprKind::ArraySized {
                element_type: resolved_elem,
                size: Box::new(typed_size),
            },
            InferType::Array(Box::new(elem_ty)),
        )
    }

    pub(super) fn infer_vec_literal(
        &mut self,
        element_type: &Option<TypeAnnotation>,
        elements: &[Expr],
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_elements: Vec<TypedExpr> = elements.iter().map(|e| self.infer_expr(e)).collect();

        let (elem_ty, resolved_elem) = if let Some(ann) = element_type {
            let ty = self.type_from_annotation(ann);
            let resolved = ResolvedType::from_infer_type(&ty);
            (ty, Some(resolved))
        } else if typed_elements.is_empty() {
            (self.type_gen.fresh(), None)
        } else {
            let first_ty = typed_elements[0].ty.clone();
            for elem in typed_elements.iter().skip(1) {
                self.constraints.push(Constraint::equal(
                    elem.ty.clone(),
                    first_ty.clone(),
                    elem.span,
                    ConstraintReason::ArrayElement,
                ));
            }
            (first_ty, None)
        };

        (
            TypedExprKind::VecLiteral {
                element_type: resolved_elem,
                elements: typed_elements,
            },
            InferType::Vec(Box::new(elem_ty)),
        )
    }

    pub(super) fn infer_index_expr(
        &mut self,
        object: &Expr,
        index: &Expr,
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_object = self.infer_expr(object);
        let typed_index = self.infer_expr(index);

        self.constraints.push(Constraint::equal(
            typed_index.ty.clone(),
            InferType::Int,
            index.span,
            ConstraintReason::ArrayIndex,
        ));

        let elem_ty = match &typed_object.ty {
            InferType::Array(inner) => (**inner).clone(),
            InferType::Vec(inner) => (**inner).clone(),
            InferType::Dynamic => InferType::Dynamic,
            InferType::Var(_) => self.type_gen.fresh(),
            _ => InferType::Dynamic,
        };

        (
            TypedExprKind::Index {
                object: Box::new(typed_object),
                index: Box::new(typed_index),
            },
            elem_ty,
        )
    }

    pub(super) fn infer_index_assign_expr(
        &mut self,
        object: &Expr,
        index: &Expr,
        value: &Expr,
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_object = self.infer_expr(object);
        let typed_index = self.infer_expr(index);
        let typed_value = self.infer_expr(value);

        self.constraints.push(Constraint::equal(
            typed_index.ty.clone(),
            InferType::Int,
            index.span,
            ConstraintReason::ArrayIndex,
        ));

        let result_ty = typed_value.ty.clone();

        (
            TypedExprKind::IndexAssign {
                object: Box::new(typed_object),
                index: Box::new(typed_index),
                value: Box::new(typed_value),
            },
            result_ty,
        )
    }

    pub(super) fn infer_slice_expr(
        &mut self,
        object: &Expr,
        range: &Expr,
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_object = self.infer_expr(object);
        let typed_range = self.infer_expr(range);
        let result_ty = typed_object.ty.clone();

        (
            TypedExprKind::Slice {
                object: Box::new(typed_object),
                range: Box::new(typed_range),
            },
            result_ty,
        )
    }

    pub(super) fn infer_range_expr(
        &mut self,
        start: &Option<Box<Expr>>,
        end: &Option<Box<Expr>>,
        inclusive: bool,
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_start = start.as_ref().map(|e| Box::new(self.infer_expr(e)));
        let typed_end = end.as_ref().map(|e| Box::new(self.infer_expr(e)));

        if let Some(ref s) = typed_start {
            self.constraints.push(Constraint::equal(
                s.ty.clone(),
                InferType::Int,
                s.span,
                ConstraintReason::RangeBound,
            ));
        }
        if let Some(ref e) = typed_end {
            self.constraints.push(Constraint::equal(
                e.ty.clone(),
                InferType::Int,
                e.span,
                ConstraintReason::RangeBound,
            ));
        }

        (
            TypedExprKind::Range {
                start: typed_start,
                end: typed_end,
                inclusive,
            },
            InferType::Range,
        )
    }
}

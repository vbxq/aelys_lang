use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::{InferType, ResolvedType};
use aelys_syntax::{Expr, Span, TypeAnnotation};

impl TypeInference {
    pub(super) fn infer_array_literal(
        &mut self,
        elements: &[Expr],
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_elements: Vec<TypedExpr> = elements.iter().map(|e| self.infer_expr(e)).collect();

        let elem_ty = if typed_elements.is_empty() {
            self.type_gen.fresh()
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
            first_ty
        };

        let len = typed_elements.len() as u64;
        (
            TypedExprKind::ArrayLiteral {
                elements: typed_elements,
            },
            InferType::Array(Box::new(elem_ty), Some(len)),
        )
    }

    pub(super) fn infer_array_sized(
        &mut self,
        size: &Expr,
        fill_value: Option<&Expr>,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_size = self.infer_expr(size);

        self.constraints.push(Constraint::equal(
            typed_size.ty.clone(),
            InferType::I64,
            span,
            ConstraintReason::ArrayIndex,
        ));

        // Extract const size for the array length
        let array_len = match &typed_size.kind {
            TypedExprKind::Int(n) if *n >= 0 => Some(*n as u64),
            _ => None,
        };

        let typed_fill = fill_value.map(|fv| Box::new(self.infer_expr(fv)));

        let elem_ty = if let Some(ref fv) = typed_fill {
            fv.ty.clone()
        } else {
            InferType::Dynamic
        };

        (
            TypedExprKind::ArraySized {
                size: Box::new(typed_size),
                fill_value: typed_fill,
            },
            InferType::Array(Box::new(elem_ty), array_len),
        )
    }

    pub(super) fn infer_vec_literal(
        &mut self,
        element_type: &Option<TypeAnnotation>,
        elements: &[Expr],
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let mut typed_elements: Vec<TypedExpr> =
            elements.iter().map(|e| self.infer_expr(e)).collect();

        let (elem_ty, resolved_elem) = if let Some(ann) = element_type {
            let ty = self.type_from_annotation(ann);
            // verifies elements of vec<T>[...]
            // TODO: what if the programmer want to mix up data in Vec
            // we should probably allow that at some point, make it Dynamic ?
            for elem in &mut typed_elements {
                self.try_narrow_literal(elem, &ty);
                self.constraints.push(Constraint::equal(
                    elem.ty.clone(),
                    ty.clone(),
                    elem.span,
                    ConstraintReason::ArrayElement,
                ));
            }

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
            InferType::I64,
            index.span,
            ConstraintReason::ArrayIndex,
        ));

        let elem_ty = match &typed_object.ty {
            InferType::Array(inner, _) => (**inner).clone(),
            InferType::Vec(inner) => (**inner).clone(),
            InferType::String => InferType::String,
            InferType::Dynamic => InferType::Dynamic,
            InferType::Var(_) => self.type_gen.fresh(),
            other => {
                self.errors.push(TypeError::member_access(
                    format!("index operation on non-indexable type {}", other),
                    _span,
                ));
                InferType::Dynamic
            }
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
        let mut typed_value = self.infer_expr(value);

        self.constraints.push(Constraint::equal(
            typed_index.ty.clone(),
            InferType::I64,
            index.span,
            ConstraintReason::ArrayIndex,
        ));

        // narrow the assigned value and constrain it to match element type.
        match &typed_object.ty {
            InferType::Array(elem_ty, _) | InferType::Vec(elem_ty) => {
                self.try_narrow_literal(&mut typed_value, elem_ty);
                self.constraints.push(Constraint::equal(
                    typed_value.ty.clone(),
                    (**elem_ty).clone(),
                    _span,
                    ConstraintReason::ArrayElement,
                ));
            }
            InferType::String => {
                self.errors.push(TypeError::member_access(
                    "index assignment on non-indexable type string".to_string(),
                    _span,
                ));
            }
            InferType::Dynamic | InferType::Var(_) => {
                // permissive: Dynamic accepts anything, Var may resolve later.
            }
            other => {
                self.errors.push(TypeError::member_access(
                    format!("index assignment on non-indexable type {}", other),
                    _span,
                ));
            }
        }

        (
            TypedExprKind::IndexAssign {
                object: Box::new(typed_object),
                index: Box::new(typed_index),
                value: Box::new(typed_value),
            },
            InferType::Null,
        )
    }

    pub(super) fn infer_slice_expr(
        &mut self,
        object: &Expr,
        range: &Expr,
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_object = self.infer_expr(object);
        let typed_range = self.infer_expr(range);
        self.errors.push(TypeError::member_access(
            "slice expressions are not supported yet".to_string(),
            span,
        ));

        (
            TypedExprKind::Slice {
                object: Box::new(typed_object),
                range: Box::new(typed_range),
            },
            InferType::Dynamic,
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
                InferType::I64,
                s.span,
                ConstraintReason::RangeBound,
            ));
        }
        if let Some(ref e) = typed_end {
            self.constraints.push(Constraint::equal(
                e.ty.clone(),
                InferType::I64,
                e.span,
                ConstraintReason::RangeBound,
            ));
        }
        self.errors.push(TypeError::member_access(
            "range expressions are not supported yet".to_string(),
            _span,
        ));

        (
            TypedExprKind::Range {
                start: typed_start,
                end: typed_end,
                inclusive,
            },
            InferType::Dynamic,
        )
    }
}

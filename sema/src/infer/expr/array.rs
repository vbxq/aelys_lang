use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorSuggestion};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::{InferType, ResolvedType};
use aelys_syntax::{Expr, ExprKind, Span, TypeAnnotation};

impl TypeInference {
    fn reject_rc_aggregate_elements(&mut self, elements: &[TypedExpr], kind: &str) {
        for elem in elements {
            if elem.ty.is_rc()
                || elem.ty.contains_rc()
                || self.type_table.contains_rc_nominal(&elem.ty)
            {
                self.errors.push(TypeError::rc_out_of_surface(
                    format!(
                        "element of {} is a value of type `{}` which embeds an `Rc<T>`; \
                         storing an Rc inside an array/vec is not supported in Stage 1",
                        kind, elem.ty
                    ),
                    elem.span,
                ));
            }
        }
    }

    // same hazard one container down: an aggregate element that owns a vec buffer is copied
    fn reject_vec_aggregate_elements(&mut self, elements: &[TypedExpr], kind: &str) {
        for elem in elements {
            if self.type_table.contains_vec_by_value(&elem.ty) {
                self.errors.push(TypeError::vec_out_of_surface(
                    format!(
                        "element of {} has type `{}`, which holds a `Vec<T>` by value; \
                         a Vec inside a Vec/array is not supported yet (the buffer would be \
                         shared without a retain, the transitive Vec retain/release is not \
                         implemented)",
                        kind, elem.ty
                    ),
                    elem.span,
                ));
            }
        }
    }

    pub(super) fn infer_array_literal(
        &mut self,
        elements: &[Expr],
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_elements: Vec<TypedExpr> = elements.iter().map(|e| self.infer_expr(e)).collect();

        self.reject_rc_aggregate_elements(&typed_elements, "array literal");
        self.reject_vec_aggregate_elements(&typed_elements, "array literal");

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

        let array_len = match &typed_size.kind {
            TypedExprKind::Int(n) if *n >= 0 => Some(*n as u64),
            _ => None,
        };

        let typed_fill = fill_value.map(|fv| Box::new(self.infer_expr(fv)));

        if let Some(ref fv) = typed_fill {
            if fv.ty.is_rc() || fv.ty.contains_rc() || self.type_table.contains_rc_nominal(&fv.ty) {
                self.errors.push(TypeError::rc_out_of_surface(
                    format!(
                        "fill value of array `[_; N]` is a value of type `{}` which embeds an \
                         `Rc<T>`; storing an Rc inside an array is not supported yet",
                        fv.ty
                    ),
                    fv.span,
                ));
            }
            if self.type_table.contains_vec_by_value(&fv.ty) {
                self.errors.push(TypeError::vec_out_of_surface(
                    format!(
                        "fill value of array `[_; N]` has type `{}`, which holds a `Vec<T>` \
                         by value; a Vec inside a Vec/array is not supported yet (the buffer \
                         would be shared without a retain, the transitive Vec retain/release \
                         is not implemented)",
                        fv.ty
                    ),
                    fv.span,
                ));
            }
        }

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

        self.reject_rc_aggregate_elements(&typed_elements, "vec literal");
        self.reject_vec_aggregate_elements(&typed_elements, "vec literal");

        let (elem_ty, resolved_elem) = if let Some(ann) = element_type {
            let ty = self.type_from_annotation(ann);
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
            InferType::Slice { elem, .. } => (**elem).clone(),
            InferType::String => InferType::String,
            InferType::Dynamic => InferType::Dynamic,
            InferType::Var(_) => self.type_gen.fresh(),
            _other => InferType::Dynamic,
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

        // a slice write is governed by the slice referent, not the binding mutability
        if !matches!(typed_object.ty, InferType::Slice { .. }) {
            if let ExprKind::Identifier(ref name) = object.kind {
                if !self.env.is_mutable(name) {
                    let binding_span = self.env.lookup_binding_span(name);
                    let suggestion = binding_span.map(|bs| {
                        let insert_offset = bs.start + 4;
                        let insert_span =
                            Span::new(insert_offset, insert_offset, bs.line, bs.column + 4);
                        TypeErrorSuggestion {
                            message: "make the binding mutable".to_string(),
                            span: insert_span,
                            new_text: "mut ".to_string(),
                        }
                    });
                    self.errors.push(TypeError::assign_to_immutable(
                        name.to_string(),
                        _span,
                        binding_span,
                        suggestion,
                    ));
                }
            }
        }

        self.constraints.push(Constraint::equal(
            typed_index.ty.clone(),
            InferType::I64,
            index.span,
            ConstraintReason::ArrayIndex,
        ));

        // r1-r3 already refuse the construction forms; this is the fail-closed backstop
        if matches!(typed_object.ty, InferType::Array(_, _) | InferType::Vec(_))
            && self.type_table.contains_vec_by_value(&typed_value.ty)
        {
            self.errors.push(TypeError::vec_out_of_surface(
                format!(
                    "value stored into an element of `{}` has type `{}`, which holds a `Vec<T>` \
                     by value; a Vec inside a Vec/array is not supported yet (the buffer would \
                     be shared without a retain, the transitive Vec retain/release is not \
                     implemented)",
                    typed_object.ty, typed_value.ty
                ),
                typed_value.span,
            ));
        }

        match &typed_object.ty {
            InferType::Array(elem_ty, _)
            | InferType::Vec(elem_ty)
            | InferType::Slice { elem: elem_ty, .. } => {
                self.try_narrow_literal(&mut typed_value, elem_ty);
                if typed_value.ty != **elem_ty && typed_value.ty.can_implicit_widen_to(elem_ty) {
                    let vspan = typed_value.span;
                    let original = std::mem::replace(
                        &mut typed_value,
                        TypedExpr {
                            kind: TypedExprKind::Null,
                            ty: InferType::Null,
                            span: vspan,
                        },
                    );
                    typed_value = TypedExpr {
                        kind: TypedExprKind::Cast {
                            expr: Box::new(original),
                            target: (**elem_ty).clone(),
                        },
                        ty: (**elem_ty).clone(),
                        span: vspan,
                    };
                }
                self.constraints.push(Constraint::flows(
                    typed_value.ty.clone(),
                    (**elem_ty).clone(),
                    _span,
                    ConstraintReason::ArrayElement,
                ));
            }
            InferType::String | InferType::Dynamic | InferType::Var(_) | _ => {
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
        _span: Span,
    ) -> (TypedExprKind, InferType) {
        let typed_object = self.infer_expr(object);

        // real sliceability error is reported post-substitution in validate.rs
        let elem_ty = match &typed_object.ty {
            InferType::Array(inner, _) => (**inner).clone(),
            InferType::Vec(inner) => (**inner).clone(),
            InferType::Slice { elem, .. } => (**elem).clone(),
            InferType::Dynamic => InferType::Dynamic,
            InferType::Var(_) => self.type_gen.fresh(),
            _other => InferType::Dynamic,
        };

        let typed_range = self.infer_slice_range(range);
        let mutable = {
            let env = &self.env;
            crate::place_spine::place_is_writable(&typed_object, &|n: &str| env.is_mutable(n))
        };

        (
            TypedExprKind::Slice {
                object: Box::new(typed_object),
                range: Box::new(typed_range),
            },
            InferType::Slice {
                elem: Box::new(elem_ty),
                mutable,
            },
        )
    }

    fn infer_slice_range(&mut self, range: &Expr) -> TypedExpr {
        match &range.kind {
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => {
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
                TypedExpr {
                    kind: TypedExprKind::Range {
                        start: typed_start,
                        end: typed_end,
                        inclusive: *inclusive,
                    },
                    ty: InferType::Range,
                    span: range.span,
                }
            }
            _ => self.infer_expr(range),
        }
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

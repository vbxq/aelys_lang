mod array;
mod assign;
mod binary;
mod call;
mod catch_expr;
mod if_expr;
mod lambda;
mod match_expr;
mod member;
mod primary;
mod try_expr;

use super::{LiteralInit, TypeInference};
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedFmtStringPart, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::{BinaryOp, Expr, ExprKind};

impl TypeInference {
    pub(super) fn infer_expr(&mut self, expr: &Expr) -> TypedExpr {
        self.depth += 1;
        if self.depth > super::MAX_INFERENCE_DEPTH {
            self.errors.push(TypeError::recursion_limit(expr.span));
            self.depth -= 1;
            return TypedExpr {
                kind: TypedExprKind::Null,
                ty: InferType::Dynamic,
                span: expr.span,
            };
        }

        let (kind, ty) = match &expr.kind {
            ExprKind::Int(n) => (TypedExprKind::Int(*n), InferType::I64),
            ExprKind::Float(f) => (TypedExprKind::Float(*f), InferType::F64),
            ExprKind::Bool(b) => (TypedExprKind::Bool(*b), InferType::Bool),
            ExprKind::String(s) => (TypedExprKind::String(s.clone()), InferType::String),
            ExprKind::Char(cp) => (TypedExprKind::Char(*cp), InferType::Char),
            ExprKind::FmtString(parts) => {
                let typed_parts = parts
                    .iter()
                    .map(|p| match p {
                        aelys_syntax::FmtStringPart::Literal(s) => {
                            TypedFmtStringPart::Literal(s.clone())
                        }
                        aelys_syntax::FmtStringPart::Expr(e) => {
                            TypedFmtStringPart::Expr(Box::new(self.infer_expr(e)))
                        }
                        aelys_syntax::FmtStringPart::Placeholder => TypedFmtStringPart::Placeholder,
                    })
                    .collect();
                (TypedExprKind::FmtString(typed_parts), InferType::String)
            }
            ExprKind::Null => (TypedExprKind::Null, InferType::Null),
            ExprKind::Identifier(name) => self.infer_identifier_expr(name, expr.span),
            ExprKind::Binary { left, op, right } => {
                let mut typed_left = self.infer_expr(left);
                let mut typed_right = self.infer_expr(right);
                let is_shift = matches!(op, BinaryOp::Shl | BinaryOp::Shr);
                let left_lit = self.is_literal_like(&typed_left);
                let right_lit = self.is_literal_like(&typed_right);
                if is_shift {
                    let width = typed_left.ty.clone();
                    self.fit_shift_amount(&mut typed_right, &width);
                } else if !left_lit && right_lit {
                    let width = typed_left.ty.clone();
                    if width.is_integer() || width.is_float() {
                        self.try_narrow_literal(&mut typed_right, &width);
                    }
                } else if left_lit && !right_lit {
                    let width = typed_right.ty.clone();
                    if width.is_integer() || width.is_float() {
                        self.try_narrow_literal(&mut typed_left, &width);
                    }
                }
                if !is_shift
                    && typed_left.ty != typed_right.ty
                    && typed_left.ty.can_implicit_widen_to(&typed_right.ty)
                {
                    let span = typed_left.span;
                    let original = std::mem::replace(
                        &mut typed_left,
                        TypedExpr {
                            kind: TypedExprKind::Null,
                            ty: InferType::Null,
                            span,
                        },
                    );
                    typed_left = TypedExpr {
                        kind: TypedExprKind::Cast {
                            expr: Box::new(original),
                            target: typed_right.ty.clone(),
                        },
                        ty: typed_right.ty.clone(),
                        span,
                    };
                } else if !is_shift
                    && typed_left.ty != typed_right.ty
                    && typed_right.ty.can_implicit_widen_to(&typed_left.ty)
                {
                    let span = typed_right.span;
                    let original = std::mem::replace(
                        &mut typed_right,
                        TypedExpr {
                            kind: TypedExprKind::Null,
                            ty: InferType::Null,
                            span,
                        },
                    );
                    typed_right = TypedExpr {
                        kind: TypedExprKind::Cast {
                            expr: Box::new(original),
                            target: typed_left.ty.clone(),
                        },
                        ty: typed_left.ty.clone(),
                        span,
                    };
                }

                let result_type = self.infer_binary_op(*op, &typed_left, &typed_right, expr.span);

                (
                    TypedExprKind::Binary {
                        left: Box::new(typed_left),
                        op: *op,
                        right: Box::new(typed_right),
                    },
                    result_type,
                )
            }
            ExprKind::Unary { op, operand } => {
                let typed_operand = self.infer_expr(operand);
                let result_type = self.infer_unary_op(*op, &typed_operand, expr.span);

                (
                    TypedExprKind::Unary {
                        op: *op,
                        operand: Box::new(typed_operand),
                    },
                    result_type,
                )
            }
            ExprKind::And { left, right } => self.infer_logical_expr("and", left, right, expr),
            ExprKind::Or { left, right } => self.infer_logical_expr("or", left, right, expr),
            ExprKind::Call { callee, args } => self.infer_call_expr(callee, args, expr.span),
            ExprKind::Assign { name, value } => self.infer_assign_expr(name, value, expr.span),
            ExprKind::Grouping(inner) => {
                let typed_inner = self.infer_expr(inner);
                let ty = typed_inner.ty.clone();
                (TypedExprKind::Grouping(Box::new(typed_inner)), ty)
            }
            ExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.infer_if_expr(condition, then_branch, else_branch, expr.span),
            ExprKind::Lambda {
                params,
                return_type,
                body,
            } => self.infer_lambda_expr(params, return_type.as_ref(), body, expr.span),
            ExprKind::Member { object, member } => {
                self.infer_member_expr(object, member, expr.span)
            }
            ExprKind::ArrayLiteral { elements } => self.infer_array_literal(elements, expr.span),
            ExprKind::ArraySized { size, fill_value } => {
                self.infer_array_sized(size, fill_value.as_deref(), expr.span)
            }
            ExprKind::VecLiteral {
                element_type,
                elements,
            } => self.infer_vec_literal(element_type, elements, expr.span),
            ExprKind::Index { object, index } => self.infer_index_expr(object, index, expr.span),
            ExprKind::IndexAssign {
                object,
                index,
                value,
            } => self.infer_index_assign_expr(object, index, value, expr.span),
            ExprKind::FieldAssign {
                object,
                field,
                value,
            } => self.infer_field_assign_expr(object, field, value, expr.span),
            ExprKind::Range {
                start,
                end,
                inclusive,
            } => self.infer_range_expr(start, end, *inclusive, expr.span),
            ExprKind::Slice { object, range } => self.infer_slice_expr(object, range, expr.span),
            ExprKind::Reference { mutable, operand } => {
                let mut bare = &**operand;
                while let ExprKind::Grouping(inner) = &bare.kind {
                    bare = inner;
                }
                let names_binding = matches!(bare.kind, ExprKind::Identifier(_));
                let saved = self.keep_binding_use;
                self.keep_binding_use = saved || names_binding;
                let typed_operand = self.infer_expr(operand);
                self.keep_binding_use = saved;
                if *mutable {
                    if let ExprKind::Identifier(name) = &operand.kind {
                        if self.env.lookup_local(name).is_some() && !self.env.is_mutable(name) {
                            self.errors.push(TypeError::mut_ref_immutable_binding(
                                name.clone(),
                                expr.span,
                            ));
                        }
                    }
                }
                let referent = typed_operand.ty.clone();
                (
                    TypedExprKind::Reference {
                        mutable: *mutable,
                        operand: Box::new(typed_operand),
                    },
                    InferType::Ref {
                        referent: Box::new(referent),
                        mutable: *mutable,
                    },
                )
            }
            ExprKind::Deref(operand) => {
                let typed_operand = self.infer_expr(operand);
                let result_ty = match &typed_operand.ty {
                    InferType::Ref { referent, .. } => (**referent).clone(),
                    InferType::Var(_) => self.defer_read(
                        &typed_operand.ty,
                        crate::infer::projection::ProjKind::Deref,
                        expr.span,
                    ),
                    InferType::Dynamic => InferType::Dynamic,
                    other => {
                        self.errors.push(TypeError::member_access(
                            format!(
                                "cannot dereference a non-reference value of type `{}`",
                                other
                            ),
                            expr.span,
                        ));
                        InferType::Dynamic
                    }
                };
                (TypedExprKind::Deref(Box::new(typed_operand)), result_ty)
            }
            ExprKind::DerefAssign { target, value } => {
                let typed_target = self.infer_expr(target);
                let typed_value = self.infer_expr(value);
                if let InferType::Ref { referent, .. } = &typed_target.ty {
                    self.constraints.push(Constraint::flows_into(
                        (**referent).clone(),
                        typed_value.ty.clone(),
                        expr.span,
                        ConstraintReason::Other("write through reference".to_string()),
                    ));
                } else if matches!(typed_target.ty, InferType::Var(_)) {
                    self.defer_write(
                        &typed_target.ty,
                        crate::infer::projection::ProjKind::DerefWrite,
                        &typed_value.ty,
                        expr.span,
                    );
                }
                (
                    TypedExprKind::DerefAssign {
                        target: Box::new(typed_target),
                        value: Box::new(typed_value),
                    },
                    InferType::Null,
                )
            }
            ExprKind::StructLiteral { name, fields } => {
                self.infer_struct_literal(name, fields, expr.span)
            }
            ExprKind::Cast {
                expr: inner,
                target,
            } => {
                let typed_inner = self.infer_expr(inner);
                let target_ty = self.type_from_annotation(target);
                (
                    TypedExprKind::Cast {
                        expr: Box::new(typed_inner),
                        target: target_ty.clone(),
                    },
                    target_ty,
                )
            }
            ExprKind::EnumVariant {
                enum_name,
                variant,
                args,
                unqualified,
            } => {
                if *unqualified
                    && let Some(refusal) =
                        self.unqualified_variant_refusal(enum_name, variant, expr.span)
                {
                    self.errors.push(refusal);
                    for arg in args {
                        self.infer_expr(arg);
                    }
                    (TypedExprKind::Null, InferType::Dynamic)
                } else {
                    if *unqualified {
                        self.unqualified_variant_spans
                            .insert((expr.span.start, expr.span.end));
                    }
                    self.infer_enum_variant(enum_name, variant, args, expr.span)
                }
            }
            ExprKind::Match { scrutinee, arms } => {
                self.infer_match_expr(scrutinee, arms, expr.span)
            }
            ExprKind::Block { stmts, tail } => {
                self.env.push_scope();
                let typed_stmts = self.infer_stmts(stmts);
                let typed_tail = self.infer_expr(tail);
                self.env.pop_scope();
                let ty = if typed_stmts
                    .iter()
                    .any(|s| matches!(s.kind, TypedStmtKind::Return(_)))
                {
                    InferType::Never
                } else {
                    typed_tail.ty.clone()
                };
                (
                    TypedExprKind::Block {
                        stmts: typed_stmts,
                        tail: Box::new(typed_tail),
                    },
                    ty,
                )
            }
            ExprKind::Try(inner) => self.infer_try_expr(inner, expr.span),
            ExprKind::Catch { scrutinee, handler } => {
                self.infer_catch_expr(scrutinee, handler, expr.span)
            }
            ExprKind::Unsafe(inner) => {
                self.unsafe_depth += 1;
                let t = self.infer_expr(inner);
                self.unsafe_depth -= 1;
                (t.kind, t.ty)
            }
        };

        self.depth -= 1;
        let (kind, ty) = match &kind {
            TypedExprKind::Identifier(name) if !self.keep_binding_use => match self
                .env
                .lookup_binding_id(name)
                .and_then(|id| self.literal_init_vars.get(&id))
            {
                Some(LiteralInit::Int(v)) => (TypedExprKind::Int(*v), InferType::I64),
                Some(LiteralInit::Float(v)) => (TypedExprKind::Float(*v), InferType::F64),
                None => (kind, ty),
            },
            _ => (kind, ty),
        };
        TypedExpr {
            kind,
            ty,
            span: expr.span,
        }
    }

    pub(super) fn is_literal_like(&self, expr: &TypedExpr) -> bool {
        match &expr.kind {
            TypedExprKind::Int(_) | TypedExprKind::Float(_) => true,
            TypedExprKind::Grouping(inner) => self.is_literal_like(inner),
            TypedExprKind::Unary {
                op: aelys_syntax::UnaryOp::Neg | aelys_syntax::UnaryOp::BitNot,
                operand,
            } => self.is_literal_like(operand),
            TypedExprKind::Binary { left, op, right } => match op {
                BinaryOp::Shl | BinaryOp::Shr => self.is_literal_like(left),
                BinaryOp::Add
                | BinaryOp::Sub
                | BinaryOp::Mul
                | BinaryOp::Div
                | BinaryOp::Mod
                | BinaryOp::BitAnd
                | BinaryOp::BitOr
                | BinaryOp::BitXor => self.is_literal_like(left) && self.is_literal_like(right),
                _ => false,
            },
            TypedExprKind::If {
                then_branch,
                else_branch,
                ..
            } => self.is_literal_like(then_branch) && self.is_literal_like(else_branch),
            TypedExprKind::Match { arms, .. } => {
                !arms.is_empty() && arms.iter().all(|arm| self.is_literal_like(&arm.body))
            }
            TypedExprKind::Block { tail, .. } => {
                !matches!(expr.ty, InferType::Never) && self.is_literal_like(tail)
            }
            _ => false,
        }
    }

    fn fit_shift_amount(&mut self, amount: &mut TypedExpr, width: &InferType) {
        if !width.is_integer() {
            return;
        }
        if self.is_literal_like(amount) {
            self.try_narrow_literal(amount, width);
            return;
        }
        if amount.ty != *width && amount.ty.is_integer() {
            let span = amount.span;
            let original = std::mem::replace(
                amount,
                TypedExpr {
                    kind: TypedExprKind::Null,
                    ty: InferType::Null,
                    span,
                },
            );
            *amount = TypedExpr {
                kind: TypedExprKind::Cast {
                    expr: Box::new(original),
                    target: width.clone(),
                },
                ty: width.clone(),
                span,
            };
        }
    }

    fn infer_logical_expr(
        &mut self,
        op_label: &str,
        left: &Expr,
        right: &Expr,
        _expr: &Expr,
    ) -> (TypedExprKind, InferType) {
        let typed_left = self.infer_expr(left);
        let typed_right = self.infer_expr(right);

        self.constraints.push(Constraint::equal(
            typed_left.ty.clone(),
            InferType::Bool,
            left.span,
            ConstraintReason::BinaryOp {
                op: op_label.to_string(),
            },
        ));
        self.constraints.push(Constraint::equal(
            typed_right.ty.clone(),
            InferType::Bool,
            right.span,
            ConstraintReason::BinaryOp {
                op: op_label.to_string(),
            },
        ));

        (
            if op_label == "and" {
                TypedExprKind::And {
                    left: Box::new(typed_left),
                    right: Box::new(typed_right),
                }
            } else {
                TypedExprKind::Or {
                    left: Box::new(typed_left),
                    right: Box::new(typed_right),
                }
            },
            InferType::Bool,
        )
    }

    fn try_extract_int_value(expr: &TypedExpr) -> Option<i64> {
        match &expr.kind {
            TypedExprKind::Int(v) => Some(*v),
            TypedExprKind::Unary {
                op: aelys_syntax::UnaryOp::Neg,
                operand,
            } => {
                if let TypedExprKind::Int(v) = &operand.kind {
                    v.checked_neg()
                } else {
                    None
                }
            }
            _ => None,
        }
    }

    fn const_int_value(expr: &TypedExpr) -> Option<i64> {
        match &expr.kind {
            TypedExprKind::Int(v) => Some(*v),
            TypedExprKind::Grouping(inner) => Self::const_int_value(inner),
            TypedExprKind::Unary {
                op: aelys_syntax::UnaryOp::Neg,
                operand,
            } => Self::const_int_value(operand)?.checked_neg(),
            TypedExprKind::Binary { left, op, right } => Self::compute_binop_int_result(
                *op,
                Self::const_int_value(left)?,
                Self::const_int_value(right)?,
            ),
            _ => None,
        }
    }

    fn retype_literal(expr: &mut TypedExpr, target_ty: &InferType) {
        expr.ty = target_ty.clone();
        match &mut expr.kind {
            TypedExprKind::Grouping(inner) => Self::retype_literal(inner, target_ty),
            TypedExprKind::Unary { operand, .. } => Self::retype_literal(operand, target_ty),
            TypedExprKind::Binary { left, right, .. } => {
                Self::retype_literal(left, target_ty);
                Self::retype_literal(right, target_ty);
            }
            _ => {}
        }
    }

    fn compute_binop_int_result(op: BinaryOp, left: i64, right: i64) -> Option<i64> {
        match op {
            BinaryOp::Add => left.checked_add(right),
            BinaryOp::Sub => left.checked_sub(right),
            BinaryOp::Mul => left.checked_mul(right),
            _ => None, // div, mod, comparisons, shifts: not validated at narrowing time
        }
    }

    pub(super) fn try_narrow_literal(
        &mut self,
        expr: &mut TypedExpr,
        target_ty: &InferType,
    ) -> bool {
        if let InferType::Slice { mutable: false, .. } = target_ty {
            Self::demote_slice_view(expr);
        }

        let lit_like = self.is_literal_like(expr);
        if lit_like
            && target_ty.is_integer()
            && *target_ty != InferType::I64
            && !matches!(expr.kind, TypedExprKind::Int(_))
            && let Some(value) = Self::const_int_value(expr)
        {
            if InferType::int_fits(value, target_ty) {
                Self::retype_literal(expr, target_ty);
                return true;
            }
            self.errors.push(TypeError {
                kind: TypeErrorKind::Mismatch {
                    expected: target_ty.clone(),
                    found: InferType::I64,
                },
                span: expr.span,
                reason: ConstraintReason::IntLiteralOverflow {
                    value,
                    target: target_ty.clone(),
                },
                secondary_spans: Vec::new(),
                help: None,
                suggestion: None,
            });
            return false;
        }
        // extract values needed for matching before taking mutable borrows.
        let int_val = if let TypedExprKind::Int(v) = &expr.kind {
            Some(*v)
        } else {
            None
        };
        let float_val = if let TypedExprKind::Float(v) = &expr.kind {
            Some(*v)
        } else {
            None
        };

        if let Some(value) = int_val {
            if target_ty.is_integer() && *target_ty != InferType::I64 {
                if InferType::int_fits(value, target_ty) {
                    expr.ty = target_ty.clone();
                    return true;
                } else {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: target_ty.clone(),
                            found: InferType::I64,
                        },
                        span: expr.span,
                        reason: ConstraintReason::IntLiteralOverflow {
                            value,
                            target: target_ty.clone(),
                        },
                        secondary_spans: Vec::new(),
                        help: None,
                        suggestion: None,
                    });
                    return false;
                }
            }
        }

        if let Some(value) = float_val {
            if target_ty.is_float() && *target_ty != InferType::F64 {
                if InferType::float_fits(value, target_ty) {
                    expr.ty = target_ty.clone();
                    return true;
                } else {
                    self.errors.push(TypeError {
                        kind: TypeErrorKind::Mismatch {
                            expected: target_ty.clone(),
                            found: InferType::F64,
                        },
                        span: expr.span,
                        reason: ConstraintReason::FloatLiteralOverflow {
                            value,
                            target: target_ty.clone(),
                        },
                        secondary_spans: Vec::new(),
                        help: None,
                        suggestion: None,
                    });
                    return false;
                }
            }
        }

        if let InferType::Array(elem_ty, _) = target_ty {
            if let TypedExprKind::ArrayLiteral { ref mut elements } = expr.kind {
                let mut all_narrowed = true;
                let mut had_error = false;
                for elem in elements.iter_mut() {
                    if !self.try_narrow_literal(elem, elem_ty) {
                        had_error = true;
                    } else if elem.ty != **elem_ty && !matches!(elem.ty, InferType::Var(_)) {
                        if elem.ty.can_implicit_widen_to(elem_ty) {
                            let vspan = elem.span;
                            let original = std::mem::replace(
                                elem,
                                TypedExpr {
                                    kind: TypedExprKind::Null,
                                    ty: InferType::Null,
                                    span: vspan,
                                },
                            );
                            *elem = TypedExpr {
                                kind: TypedExprKind::Cast {
                                    expr: Box::new(original),
                                    target: (**elem_ty).clone(),
                                },
                                ty: (**elem_ty).clone(),
                                span: vspan,
                            };
                        } else {
                            all_narrowed = false;
                        }
                    }
                }
                if all_narrowed && !had_error {
                    // the annotation gives the element type, never the count: stamping its
                    let count = elements.len() as u64;
                    expr.ty = InferType::Array(elem_ty.clone(), Some(count));
                }

                return !had_error;
            }
        }

        if let InferType::Array(elem_ty, target_len) = target_ty {
            if let TypedExprKind::ArraySized { size, fill_value } = &mut expr.kind {
                // not override one it does: that is how a repeat overruns its storage
                let size_agrees = match (Self::closed_const_int(&**size), target_len) {
                    (Some(n), Some(len)) => n >= 0 && n as u64 == *len,
                    _ => true,
                };
                if let Some(fv) = fill_value {
                    if self.try_narrow_literal(fv, elem_ty) && fv.ty == **elem_ty && size_agrees {
                        expr.ty = target_ty.clone();
                        return true;
                    }
                }
                return true;
            }
        }

        if let TypedExprKind::Grouping(inner) = &mut expr.kind
            && lit_like
        {
            let ok = self.try_narrow_literal(inner, target_ty);
            if ok && inner.ty == *target_ty {
                expr.ty = target_ty.clone();
            }
            return ok;
        }

        if let TypedExprKind::Unary { operand, .. } = &mut expr.kind
            && lit_like
        {
            if expr.ty == InferType::I64 && target_ty.is_integer() && *target_ty != InferType::I64 {
                let ok = self.try_narrow_literal(operand, target_ty);
                if ok && operand.ty == *target_ty {
                    expr.ty = target_ty.clone();
                    return true;
                }
                return ok;
            }
            if expr.ty == InferType::F64 && target_ty.is_float() && *target_ty != InferType::F64 {
                let ok = self.try_narrow_literal(operand, target_ty);
                if ok && operand.ty == *target_ty {
                    expr.ty = target_ty.clone();
                    return true;
                }
                return ok;
            }
        }

        if let TypedExprKind::Binary {
            left, right, op, ..
        } = &mut expr.kind
            && lit_like
        {
            if matches!(op, BinaryOp::Shl | BinaryOp::Shr)
                && expr.ty == InferType::I64
                && target_ty.is_integer()
                && *target_ty != InferType::I64
            {
                let ok = self.try_narrow_literal(left, target_ty);
                if ok && left.ty == *target_ty {
                    self.fit_shift_amount(right, target_ty);
                    expr.ty = target_ty.clone();
                }
                return ok;
            }
            if expr.ty == InferType::I64 && target_ty.is_integer() && *target_ty != InferType::I64 {
                let binop = *op;
                let left_ok = self.try_narrow_literal(left, target_ty);
                let right_ok = self.try_narrow_literal(right, target_ty);
                let left_narrowed = left.ty == *target_ty;
                let right_narrowed = right.ty == *target_ty;
                if left_ok && right_ok && left_narrowed && right_narrowed {
                    if let (Some(lv), Some(rv)) = (
                        Self::try_extract_int_value(left),
                        Self::try_extract_int_value(right),
                    ) {
                        if let Some(result) = Self::compute_binop_int_result(binop, lv, rv) {
                            if !InferType::int_fits(result, target_ty) {
                                self.errors.push(TypeError {
                                    kind: TypeErrorKind::Mismatch {
                                        expected: target_ty.clone(),
                                        found: InferType::I64,
                                    },
                                    span: expr.span,
                                    reason: ConstraintReason::IntLiteralOverflow {
                                        value: result,
                                        target: target_ty.clone(),
                                    },
                                    secondary_spans: Vec::new(),
                                    help: None,
                                    suggestion: None,
                                });
                                return false;
                            }
                        }
                    }
                    expr.ty = target_ty.clone();
                    return true;
                }
                if !left_ok || !right_ok {
                    return false;
                }
                return true;
            }
            if expr.ty == InferType::F64 && target_ty.is_float() && *target_ty != InferType::F64 {
                let left_ok = self.try_narrow_literal(left, target_ty);
                let right_ok = self.try_narrow_literal(right, target_ty);
                let left_narrowed = left.ty == *target_ty;
                let right_narrowed = right.ty == *target_ty;
                if left_ok && right_ok && left_narrowed && right_narrowed {
                    expr.ty = target_ty.clone();
                    return true;
                }
                if !left_ok || !right_ok {
                    return false;
                }
                return true;
            }
        }

        if let TypedExprKind::If {
            then_branch,
            else_branch,
            ..
        } = &mut expr.kind
        {
            if let InferType::Enum(..) = target_ty {
                if matches!(expr.ty, InferType::Var(_)) || matches!(expr.ty, InferType::Enum(..)) {
                    let then_ok = self.try_narrow_literal(then_branch, target_ty);
                    let else_ok = self.try_narrow_literal(else_branch, target_ty);
                    if then_ok
                        && else_ok
                        && then_branch.ty == *target_ty
                        && else_branch.ty == *target_ty
                    {
                        expr.ty = target_ty.clone();
                        return true;
                    }
                    return then_ok && else_ok;
                }
            }
            if lit_like
                && expr.ty == InferType::I64
                && target_ty.is_integer()
                && *target_ty != InferType::I64
            {
                let then_ok = self.try_narrow_literal(then_branch, target_ty);
                let else_ok = self.try_narrow_literal(else_branch, target_ty);
                let then_narrowed = then_branch.ty == *target_ty;
                let else_narrowed = else_branch.ty == *target_ty;
                if then_ok && else_ok && then_narrowed && else_narrowed {
                    expr.ty = target_ty.clone();
                    return true;
                }
                if !then_ok || !else_ok {
                    return false;
                }
                return true;
            }
            if lit_like
                && expr.ty == InferType::F64
                && target_ty.is_float()
                && *target_ty != InferType::F64
            {
                let then_ok = self.try_narrow_literal(then_branch, target_ty);
                let else_ok = self.try_narrow_literal(else_branch, target_ty);
                let then_narrowed = then_branch.ty == *target_ty;
                let else_narrowed = else_branch.ty == *target_ty;
                if then_ok && else_ok && then_narrowed && else_narrowed {
                    expr.ty = target_ty.clone();
                    return true;
                }
                if !then_ok || !else_ok {
                    return false;
                }
                return true;
            }
        }

        if let TypedExprKind::Match { arms, .. } = &mut expr.kind
            && lit_like
        {
            let is_narrowable = (expr.ty == InferType::I64
                && target_ty.is_integer()
                && *target_ty != InferType::I64)
                || (expr.ty == InferType::F64
                    && target_ty.is_float()
                    && *target_ty != InferType::F64)
                || (matches!(expr.ty, InferType::Var(_))
                    && (target_ty.is_integer() || target_ty.is_float()));
            if is_narrowable {
                let mut all_ok = true;
                let mut all_narrowed = true;
                for arm in arms.iter_mut() {
                    let ok = self.try_narrow_literal(&mut arm.body, target_ty);
                    if !ok {
                        all_ok = false;
                    }
                    if arm.body.ty != *target_ty {
                        all_narrowed = false;
                    }
                }
                if all_ok && all_narrowed {
                    expr.ty = target_ty.clone();
                    return true;
                }
                if !all_ok {
                    return false;
                }
                return true;
            }
        }

        if let TypedExprKind::Block { tail, .. } = &mut expr.kind
            && lit_like
        {
            if (expr.ty == InferType::I64 && target_ty.is_integer() && *target_ty != InferType::I64)
                || (expr.ty == InferType::F64
                    && target_ty.is_float()
                    && *target_ty != InferType::F64)
            {
                let ok = self.try_narrow_literal(tail, target_ty);
                if ok && tail.ty == *target_ty {
                    expr.ty = target_ty.clone();
                    return true;
                }
                if !ok {
                    return false;
                }
                return true;
            }
        }

        if let TypedExprKind::EnumVariant {
            args, enum_name, ..
        } = &mut expr.kind
        {
            if let InferType::Enum(target_name, target_type_args) = target_ty {
                if let InferType::Enum(expr_name, expr_type_args) = &expr.ty {
                    if enum_name == target_name
                        && expr_name == target_name
                        && target_type_args.len() == expr_type_args.len()
                    {
                        let all_expr_var = expr_type_args
                            .iter()
                            .all(|t| matches!(t, InferType::Var(_)));
                        let all_target_concrete = target_type_args.iter().all(|t| t.is_concrete());

                        if all_expr_var && all_target_concrete {
                            for target_ta in target_type_args.iter() {
                                for arg in args.iter_mut() {
                                    self.try_narrow_literal(arg, target_ta);
                                }
                            }
                            expr.ty = target_ty.clone();
                            return true;
                        }
                    }
                }
            }
        }

        true
    }

    fn demote_slice_view(expr: &mut TypedExpr) {
        if let InferType::Slice { mutable, .. } = &mut expr.ty {
            *mutable = false;
        } else {
            return;
        }
        match &mut expr.kind {
            TypedExprKind::Grouping(inner) => Self::demote_slice_view(inner),
            TypedExprKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::demote_slice_view(then_branch);
                Self::demote_slice_view(else_branch);
            }
            TypedExprKind::Match { arms, .. } => {
                for arm in arms.iter_mut() {
                    Self::demote_slice_view(&mut arm.body);
                }
            }
            TypedExprKind::Block { tail, .. } => Self::demote_slice_view(tail),
            _ => {}
        }
    }
}


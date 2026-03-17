mod array;
mod assign;
mod binary;
mod call;
mod if_expr;
mod lambda;
mod match_expr;
mod member;
mod primary;

use super::{LiteralInit, TypeInference};
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedFmtStringPart, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::{BinaryOp, Expr, ExprKind};

impl TypeInference {
    /// Infer type for an expression
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
                Self::narrow_binop_int_literals(&mut typed_left, &mut typed_right);
                Self::narrow_binop_float_literals(&mut typed_left, &mut typed_right);
                // When one operand has a concrete non-default type and the other
                // is still i64/f64/Var (e.g. a match/if/block containing literals),
                // narrow the unresolved operand to match. This handles patterns
                // like `total_i32 + match m { ... => 0 }`.
                if typed_left.ty.is_integer() && typed_left.ty != InferType::I64
                    && (typed_right.ty == InferType::I64 || matches!(typed_right.ty, InferType::Var(_)))
                {
                    self.try_narrow_literal(&mut typed_right, &typed_left.ty.clone());
                } else if typed_right.ty.is_integer() && typed_right.ty != InferType::I64
                    && (typed_left.ty == InferType::I64 || matches!(typed_left.ty, InferType::Var(_)))
                {
                    self.try_narrow_literal(&mut typed_left, &typed_right.ty.clone());
                } else if typed_left.ty.is_float() && typed_left.ty != InferType::F64
                    && (typed_right.ty == InferType::F64 || matches!(typed_right.ty, InferType::Var(_)))
                {
                    self.try_narrow_literal(&mut typed_right, &typed_left.ty.clone());
                } else if typed_right.ty.is_float() && typed_right.ty != InferType::F64
                    && (typed_left.ty == InferType::F64 || matches!(typed_left.ty, InferType::Var(_)))
                {
                    self.try_narrow_literal(&mut typed_left, &typed_right.ty.clone());
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
            ExprKind::StructLiteral { name, fields } => {
                self.infer_struct_literal(name, fields, expr.span)
            }
            ExprKind::Cast {
                expr: inner,
                target,
            } => {
                let typed_inner = self.infer_expr(inner);
                let target_ty = self.type_from_annotation(target);
                // Cast validity is checked post-substitution in validate.rs
                // to avoid duplicate errors and to work on resolved types.
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
            } => self.infer_enum_variant(enum_name, variant, args, expr.span),
            ExprKind::Match { scrutinee, arms } => {
                self.infer_match_expr(scrutinee, arms, expr.span)
            }
            ExprKind::Block { stmts, tail } => {
                let typed_stmts = self.infer_stmts(stmts);
                let typed_tail = self.infer_expr(tail);
                // If the block contains a return statement, execution never
                // reaches the tail — the block diverges and its type is Never.
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
        };

        self.depth -= 1;
        TypedExpr {
            kind,
            ty,
            span: expr.span,
        }
    }

    fn narrow_binop_int_literals(left: &mut TypedExpr, right: &mut TypedExpr) {
        let narrow = |lit: &mut TypedExpr, target: &InferType| {
            if let TypedExprKind::Int(v) = &lit.kind
                && target.is_integer()
                && *target != InferType::I64
                && InferType::int_fits(*v, target)
            {
                lit.ty = target.clone();
            }
        };
        if matches!(&left.kind, TypedExprKind::Int(_)) && right.ty.is_integer() {
            narrow(left, &right.ty.clone());
        } else if matches!(&right.kind, TypedExprKind::Int(_)) && left.ty.is_integer() {
            narrow(right, &left.ty.clone());
        }
    }

    fn narrow_binop_float_literals(left: &mut TypedExpr, right: &mut TypedExpr) {
        let narrow = |lit: &mut TypedExpr, target: &InferType| {
            if let TypedExprKind::Float(v) = &lit.kind
                && target.is_float()
                && *target != InferType::F64
                && InferType::float_fits(*v, target)
            {
                lit.ty = target.clone();
            }
        };
        if matches!(&left.kind, TypedExprKind::Float(_)) && right.ty.is_float() {
            narrow(left, &right.ty.clone());
        } else if matches!(&right.kind, TypedExprKind::Float(_)) && left.ty.is_float() {
            narrow(right, &left.ty.clone());
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

    /// Try to extract a constant integer value from a typed expression
    /// Handles `Int(v)`, `Unary(Neg, Int(v))` (which represents negative literals),
    /// and `Identifier(name)` when the variable is tracked in `literal_init_vars`
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

    /// Like `try_extract_int_value` but also resolves identifiers through `literal_init_vars` tracking.
    /// this allows overflow checks on binary ops, like `x + 28` where `x` was initialized with a known literal
    // (check for overflow_variable_on_right_side in audit_regression_tests.rs)
    fn try_extract_int_value_tracked(&self, expr: &TypedExpr) -> Option<i64> {
        if let Some(v) = Self::try_extract_int_value(expr) {
            return Some(v);
        }
        if let TypedExprKind::Identifier(name) = &expr.kind {
            if let Some(LiteralInit::Int(v)) = self.literal_init_vars.get(name) {
                return Some(*v);
            }
        }
        None
    }

    /// Compute the result of a binary operation on two known integer values
    /// uses checked arithmetic to detect rust level overflow on i64
    fn compute_binop_int_result(op: BinaryOp, left: i64, right: i64) -> Option<i64> {
        match op {
            BinaryOp::Add => left.checked_add(right),
            BinaryOp::Sub => left.checked_sub(right),
            BinaryOp::Mul => left.checked_mul(right),
            _ => None, // div, mod, comparisons, shifts: not validated at narrowing time
        }
    }

    /// Try to narrow a numeric literal to the target type.
    /// Returns true if narrowing succeeded or wasn't needed, false if it failed.
    /// Pushes an error if the literal doesn't fit in the target type.
    ///
    /// Also handles array literals: when the target is `Array(elem_ty, _)` each element is narrowed to `elem_ty`
    pub(super) fn try_narrow_literal(
        &mut self,
        expr: &mut TypedExpr,
        target_ty: &InferType,
    ) -> bool {
        // Extract values needed for matching before taking mutable borrows.
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

        // narrow array literal elements when the target type is Array(elem_ty, _)
        if let InferType::Array(elem_ty, _) = target_ty {
            if let TypedExprKind::ArrayLiteral { ref mut elements } = expr.kind {
                let mut all_narrowed = true;
                let mut had_error = false;
                for elem in elements.iter_mut() {
                    if !self.try_narrow_literal(elem, elem_ty) {
                        had_error = true;
                    } else if elem.ty != **elem_ty && !matches!(elem.ty, InferType::Var(_)) {
                        // e.element type wasn't actually narrowed, concrete mismatch (for eg String vs I64).
                        // don't set the array type, letting the constraint solver catch it
                        all_narrowed = false;
                    }
                }
                if all_narrowed && !had_error {
                    expr.ty = target_ty.clone();
                }
                // return false only for real narrowing failures (overflow)

                // for non-narrowable elements, return true so the caller can push a constraint
                return !had_error;
            }
        }

        // narrow ArraySized fill values: `[0; 3]` in an `[i32; 3]` context.
        if let InferType::Array(elem_ty, _) = target_ty {
            if let TypedExprKind::ArraySized { fill_value, .. } = &mut expr.kind {
                if let Some(fv) = fill_value {
                    if self.try_narrow_literal(fv, elem_ty) && fv.ty == **elem_ty {
                        expr.ty = target_ty.clone();
                        return true;
                    }
                }
                return true;
            }
        }

        // narrow unary expressions (for eg -1 in an i32 context) recurse into the operand so the whole expression adopts the target type
        if let TypedExprKind::Unary { operand, .. } = &mut expr.kind {
            if expr.ty == InferType::I64 && target_ty.is_integer() && *target_ty != InferType::I64 {
                let ok = self.try_narrow_literal(operand, target_ty);
                // verify the operand was actually narrowed (type matches target), not just that no error occurred
                // non-narrowable expressions return true
                // but don't change their type, so we must check both conditions
                if ok && operand.ty == *target_ty {
                    expr.ty = target_ty.clone();
                    return true;
                }
                // if narrowing pushed an error (ok=false), propagate that
                // if no error but operand wasn't narrowed, return true so caller pushes a constraint
                return ok;
            }
            if expr.ty == InferType::F64 && target_ty.is_float() && *target_ty != InferType::F64 {
                let ok = self.try_narrow_literal(operand, target_ty);
                if ok && operand.ty == *target_ty {
                    expr.ty = target_ty.clone();
                    return true;
                }
                // If narrowing pushed an error (ok=false), propagate that.
                // If no error but operand wasn't narrowed, return true so caller pushes a constraint.
                return ok;
            }
        }

        // narrow binary expressions of all-literal operands (67 + 69 in an i32 context) recursively narrow both sides so the result type matches the target.
        if let TypedExprKind::Binary {
            left, right, op, ..
        } = &mut expr.kind
        {
            if expr.ty == InferType::I64 && target_ty.is_integer() && *target_ty != InferType::I64 {
                let binop = *op;
                let left_ok = self.try_narrow_literal(left, target_ty);
                let right_ok = self.try_narrow_literal(right, target_ty);
                // verify that both operands were actually narrowed to the target type.
                //
                // non narrowable expressions (identifiers, calls, etc.) return true but don't change their type, so checking only the return value
                // would incorrectly retype the binary expression. we gotta verify the types match
                let left_narrowed = left.ty == *target_ty;
                let right_narrowed = right.ty == *target_ty;
                if left_ok && right_ok && left_narrowed && right_narrowed {
                    // when both operands are known integer literals, compute the result and verify it fits in the target type
                    // each operand individually fitting does not guarantee the result fits
                    // like, 100 + 100 = 200 overflows i8
                    if let (Some(lv), Some(rv)) = (
                        self.try_extract_int_value_tracked(left),
                        self.try_extract_int_value_tracked(right),
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
                // propagate failure
                if !left_ok || !right_ok {
                    return false;
                }
                // both returned ok but at least one wasn't narrowed: don't retype the binary expression. return true so caller pushes a constraint instead.
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

        // narrow if-else expressions where both branches are narrowable.
        // recurse into each branch so that `return if cond { 42 } else { 100 }` narrows in an i32 context.
        if let TypedExprKind::If {
            then_branch,
            else_branch,
            ..
        } = &mut expr.kind
        {
            // Narrow if-else when the target is a generic enum (e.g. Option<i32>).
            // Recurse into both branches to narrow enum variant constructions.
            if let InferType::Enum(..) = target_ty {
                if matches!(expr.ty, InferType::Var(_)) || matches!(expr.ty, InferType::Enum(..)) {
                    let then_ok = self.try_narrow_literal(then_branch, target_ty);
                    let else_ok = self.try_narrow_literal(else_branch, target_ty);
                    if then_ok && else_ok
                        && then_branch.ty == *target_ty && else_branch.ty == *target_ty
                    {
                        expr.ty = target_ty.clone();
                        return true;
                    }
                    return then_ok && else_ok;
                }
            }
            if expr.ty == InferType::I64 && target_ty.is_integer() && *target_ty != InferType::I64 {
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
            if expr.ty == InferType::F64 && target_ty.is_float() && *target_ty != InferType::F64 {
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

        // narrow match expressions: recurse into every arm body.
        // Match result types are often Var(_) (unresolved type variable) rather
        // than concrete I64/F64, so also attempt narrowing when the type is a Var.
        if let TypedExprKind::Match { arms, .. } = &mut expr.kind {
            let is_narrowable = (expr.ty == InferType::I64 && target_ty.is_integer() && *target_ty != InferType::I64)
                || (expr.ty == InferType::F64 && target_ty.is_float() && *target_ty != InferType::F64)
                || (matches!(expr.ty, InferType::Var(_)) && (target_ty.is_integer() || target_ty.is_float()));
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

        // narrow block expressions: recurse into the tail expression.
        if let TypedExprKind::Block { tail, .. } = &mut expr.kind {
            if (expr.ty == InferType::I64 && target_ty.is_integer() && *target_ty != InferType::I64)
                || (expr.ty == InferType::F64 && target_ty.is_float() && *target_ty != InferType::F64)
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

        // narrow enum variant args when the target is the same enum with concrete type params.
        // e.g. `Maybe::Just(2.5)` with target `Maybe<f32>` → narrow 2.5 to f32.
        if let TypedExprKind::EnumVariant { args, enum_name, .. } = &mut expr.kind {
            if let InferType::Enum(target_name, target_type_args) = target_ty {
                if let InferType::Enum(expr_name, expr_type_args) = &expr.ty {
                    if enum_name == target_name && expr_name == target_name
                        && target_type_args.len() == expr_type_args.len()
                    {
                        // For each type arg that is Var in expr but concrete in target,
                        // narrow the corresponding variant args.
                        // Check if all expr type args are Var (unresolved) and
                        // all target type args are concrete — if so, we can adopt
                        // the target type. For unit variants (no args), this is the
                        // only way to narrow.
                        let all_expr_var = expr_type_args.iter()
                            .all(|t| matches!(t, InferType::Var(_)));
                        let all_target_concrete = target_type_args.iter()
                            .all(|t| t.is_concrete());

                        if all_expr_var && all_target_concrete {
                            // Try to narrow data variant args to match target type params
                            for (expr_ta, target_ta) in expr_type_args.clone().iter()
                                .zip(target_type_args.iter())
                            {
                                for arg in args.iter_mut() {
                                    if arg.ty == *expr_ta || (arg.ty == InferType::I64 && target_ta.is_integer())
                                        || (arg.ty == InferType::F64 && target_ta.is_float())
                                    {
                                        self.try_narrow_literal(arg, target_ta);
                                    }
                                }
                            }
                            expr.ty = target_ty.clone();
                            return true;
                        }
                    }
                }
            }
        }

        // narrow through variable references.
        //
        // when the expression is an Identifier whose variable was initialized with a numeric literal (tracked in `literal_init_vars`), treat it
        // as if the literal appeared directly, this allows code like
        //
        //   fn f() -> i8 { let x = 100; return x }
        //
        // to narrow correctly because we know x holds the value 100
        if let TypedExprKind::Identifier(name) = &expr.kind {
            if let Some(lit) = self.literal_init_vars.get(name).cloned() {
                match lit {
                    LiteralInit::Int(value) => {
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
                    LiteralInit::Float(value) => {
                        if target_ty.is_float() && *target_ty != InferType::F64 {
                            return if InferType::float_fits(value, target_ty) {
                                expr.ty = target_ty.clone();
                                true
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
                                false
                            };
                        }
                    }
                }
            }
        }

        // not a narrowable literal, caller should handle constraint
        true
    }
}

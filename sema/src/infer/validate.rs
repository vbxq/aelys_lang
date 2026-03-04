use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError};
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedFunction, TypedStmt, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::Span;
use std::collections::HashSet;

impl TypeInference {
    /// Validate resolved typed AST invariants that are hard to encode with direct unification
    /// constraints (for example, operations initially inferred on `Var` that become concrete later).
    pub(super) fn validate_resolved_stmts(
        &mut self,
        stmts: &[TypedStmt],
        declared_type_params: &HashSet<String>,
    ) {
        let generic_scope = HashSet::new();
        for stmt in stmts {
            self.validate_stmt(stmt, &generic_scope, declared_type_params);
        }
    }

    fn validate_stmt(
        &mut self,
        stmt: &TypedStmt,
        generic_scope: &HashSet<String>,
        declared_type_params: &HashSet<String>,
    ) {
        match &stmt.kind {
            TypedStmtKind::Expression(expr) => {
                self.validate_expr(expr, generic_scope, declared_type_params);
            }
            TypedStmtKind::Let {
                initializer,
                var_type,
                ..
            } => {
                self.validate_type(var_type, stmt.span, generic_scope, declared_type_params);
                self.validate_expr(initializer, generic_scope, declared_type_params);
            }
            TypedStmtKind::Block(stmts) => {
                for inner in stmts {
                    self.validate_stmt(inner, generic_scope, declared_type_params);
                }
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.validate_expr(condition, generic_scope, declared_type_params);
                self.validate_stmt(then_branch, generic_scope, declared_type_params);
                if let Some(else_branch) = else_branch {
                    self.validate_stmt(else_branch, generic_scope, declared_type_params);
                }
            }
            TypedStmtKind::While { condition, body } => {
                self.validate_expr(condition, generic_scope, declared_type_params);
                self.validate_stmt(body, generic_scope, declared_type_params);
            }
            TypedStmtKind::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                self.validate_expr(start, generic_scope, declared_type_params);
                self.validate_expr(end, generic_scope, declared_type_params);
                if let Some(step) = step.as_ref().as_ref() {
                    self.validate_expr(step, generic_scope, declared_type_params);
                }
                self.validate_stmt(body, generic_scope, declared_type_params);
            }
            TypedStmtKind::ForEach {
                iterable,
                elem_type,
                body,
                ..
            } => {
                self.validate_expr(iterable, generic_scope, declared_type_params);
                self.validate_type(elem_type, stmt.span, generic_scope, declared_type_params);

                if !Self::is_iterable_type(&iterable.ty)
                    && !self.is_active_generic_placeholder_type(
                        &iterable.ty,
                        generic_scope,
                        declared_type_params,
                    )
                {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "for-each requires an iterable (array, vec, or string), got {}",
                            iterable.ty
                        ),
                        iterable.span,
                    ));
                }

                self.validate_stmt(body, generic_scope, declared_type_params);
            }
            TypedStmtKind::Return(Some(expr)) => {
                self.validate_expr(expr, generic_scope, declared_type_params);
            }
            TypedStmtKind::Function(func) => {
                self.validate_function(func, generic_scope, declared_type_params);
            }
            TypedStmtKind::StructDecl {
                type_params,
                fields,
                ..
            } => {
                let mut struct_scope = generic_scope.clone();
                struct_scope.extend(type_params.iter().cloned());
                for (_, ty) in fields {
                    self.validate_type(ty, stmt.span, &struct_scope, declared_type_params);
                }
            }
            TypedStmtKind::Return(None)
            | TypedStmtKind::Break
            | TypedStmtKind::Continue
            | TypedStmtKind::Needs(_) => {}
        }
    }

    fn validate_function(
        &mut self,
        func: &TypedFunction,
        parent_scope: &HashSet<String>,
        declared_type_params: &HashSet<String>,
    ) {
        let mut generic_scope = parent_scope.clone();
        generic_scope.extend(func.type_params.iter().cloned());

        for param in &func.params {
            self.validate_type(&param.ty, param.span, &generic_scope, declared_type_params);
        }
        self.validate_type(
            &func.return_type,
            func.span,
            &generic_scope,
            declared_type_params,
        );
        for (_, capture_ty) in &func.captures {
            self.validate_type(capture_ty, func.span, &generic_scope, declared_type_params);
        }
        for stmt in &func.body {
            self.validate_stmt(stmt, &generic_scope, declared_type_params);
        }
        self.validate_return_escapes_in_stmts(
            &func.body,
            &func.return_type,
            &generic_scope,
            declared_type_params,
        );
    }

    fn validate_expr(
        &mut self,
        expr: &TypedExpr,
        generic_scope: &HashSet<String>,
        declared_type_params: &HashSet<String>,
    ) {
        match &expr.kind {
            TypedExprKind::Binary { left, right, .. }
            | TypedExprKind::And { left, right }
            | TypedExprKind::Or { left, right } => {
                self.validate_expr(left, generic_scope, declared_type_params);
                self.validate_expr(right, generic_scope, declared_type_params);
            }
            TypedExprKind::Unary { operand, .. } | TypedExprKind::Grouping(operand) => {
                self.validate_expr(operand, generic_scope, declared_type_params);
            }
            TypedExprKind::Call { callee, args } => {
                self.validate_expr(callee, generic_scope, declared_type_params);
                for arg in args {
                    self.validate_expr(arg, generic_scope, declared_type_params);
                }
            }
            TypedExprKind::Assign { value, .. } => {
                self.validate_expr(value, generic_scope, declared_type_params);
            }
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.validate_expr(condition, generic_scope, declared_type_params);
                self.validate_expr(then_branch, generic_scope, declared_type_params);
                self.validate_expr(else_branch, generic_scope, declared_type_params);
            }
            TypedExprKind::Lambda(inner) => {
                self.validate_expr(inner, generic_scope, declared_type_params);
            }
            TypedExprKind::LambdaInner {
                params,
                return_type,
                body,
                captures,
            } => {
                self.validate_type(return_type, expr.span, generic_scope, declared_type_params);
                for param in params {
                    self.validate_type(&param.ty, param.span, generic_scope, declared_type_params);
                }
                for (_, capture_ty) in captures {
                    self.validate_type(capture_ty, expr.span, generic_scope, declared_type_params);
                }
                for stmt in body {
                    self.validate_stmt(stmt, generic_scope, declared_type_params);
                }
            }
            TypedExprKind::Member { object, .. } => {
                self.validate_expr(object, generic_scope, declared_type_params);
            }
            TypedExprKind::ArrayLiteral { elements }
            | TypedExprKind::VecLiteral { elements, .. } => {
                for element in elements {
                    self.validate_expr(element, generic_scope, declared_type_params);
                }
            }
            TypedExprKind::ArraySized { size, fill_value } => {
                self.validate_expr(size, generic_scope, declared_type_params);
                if let Some(fill_value) = fill_value {
                    self.validate_expr(fill_value, generic_scope, declared_type_params);
                }
            }
            TypedExprKind::Index { object, index } => {
                self.validate_expr(object, generic_scope, declared_type_params);
                self.validate_expr(index, generic_scope, declared_type_params);

                if !Self::is_indexable_type(&object.ty)
                    && !self.is_active_generic_placeholder_type(
                        &object.ty,
                        generic_scope,
                        declared_type_params,
                    )
                {
                    self.errors.push(TypeError::member_access(
                        format!("index operation on non-indexable type {}", object.ty),
                        expr.span,
                    ));
                }
            }
            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
                self.validate_expr(object, generic_scope, declared_type_params);
                self.validate_expr(index, generic_scope, declared_type_params);
                self.validate_expr(value, generic_scope, declared_type_params);

                if !Self::is_indexable_type(&object.ty)
                    && !self.is_active_generic_placeholder_type(
                        &object.ty,
                        generic_scope,
                        declared_type_params,
                    )
                {
                    self.errors.push(TypeError::member_access(
                        format!("index assignment on non-indexable type {}", object.ty),
                        expr.span,
                    ));
                }
            }
            TypedExprKind::Range { start, end, .. } => {
                if let Some(start) = start {
                    self.validate_expr(start, generic_scope, declared_type_params);
                }
                if let Some(end) = end {
                    self.validate_expr(end, generic_scope, declared_type_params);
                }
            }
            TypedExprKind::Slice { object, range } => {
                self.validate_expr(object, generic_scope, declared_type_params);
                self.validate_expr(range, generic_scope, declared_type_params);

                if !Self::is_sliceable_type(&object.ty)
                    && !self.is_active_generic_placeholder_type(
                        &object.ty,
                        generic_scope,
                        declared_type_params,
                    )
                {
                    self.errors.push(TypeError::member_access(
                        format!("slice operation on non-sliceable type {}", object.ty),
                        expr.span,
                    ));
                }
            }
            TypedExprKind::FmtString(parts) => {
                for part in parts {
                    if let crate::typed_ast::TypedFmtStringPart::Expr(inner) = part {
                        self.validate_expr(inner, generic_scope, declared_type_params);
                    }
                }
            }
            TypedExprKind::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    self.validate_expr(value, generic_scope, declared_type_params);
                }
            }
            TypedExprKind::Cast {
                expr: inner,
                target,
            } => {
                self.validate_expr(inner, generic_scope, declared_type_params);
                self.validate_type(target, expr.span, generic_scope, declared_type_params);

                if !self.is_cast_allowed_resolved(
                    &inner.ty,
                    target,
                    generic_scope,
                    declared_type_params,
                ) {
                    self.errors.push(TypeError::mismatch(
                        target.clone(),
                        inner.ty.clone(),
                        expr.span,
                        ConstraintReason::InvalidCast,
                    ));
                }
            }
            TypedExprKind::Identifier(_)
            | TypedExprKind::Int(_)
            | TypedExprKind::Float(_)
            | TypedExprKind::Bool(_)
            | TypedExprKind::String(_)
            | TypedExprKind::Null => {}
        }
    }

    fn validate_type(
        &mut self,
        ty: &InferType,
        span: Span,
        generic_scope: &HashSet<String>,
        declared_type_params: &HashSet<String>,
    ) {
        if let Some(name) = self.find_leaked_type_param(ty, generic_scope, declared_type_params) {
            self.errors.push(TypeError::member_access(
                format!(
                    "unresolved generic type parameter '{}' escaped generic context",
                    name
                ),
                span,
            ));
        }
    }

    fn find_leaked_type_param(
        &self,
        ty: &InferType,
        generic_scope: &HashSet<String>,
        declared_type_params: &HashSet<String>,
    ) -> Option<String> {
        match ty {
            InferType::Struct(name) => {
                let is_placeholder =
                    declared_type_params.contains(name) && !self.type_table.has_struct(name);
                if is_placeholder && !generic_scope.contains(name) {
                    Some(name.clone())
                } else {
                    None
                }
            }
            InferType::Function { params, ret } => params
                .iter()
                .find_map(|p| self.find_leaked_type_param(p, generic_scope, declared_type_params))
                .or_else(|| self.find_leaked_type_param(ret, generic_scope, declared_type_params)),
            InferType::Array(inner, _) | InferType::Vec(inner) => {
                self.find_leaked_type_param(inner, generic_scope, declared_type_params)
            }
            InferType::Tuple(elems) => elems
                .iter()
                .find_map(|e| self.find_leaked_type_param(e, generic_scope, declared_type_params)),
            _ => None,
        }
    }

    fn is_active_generic_placeholder_type(
        &self,
        ty: &InferType,
        generic_scope: &HashSet<String>,
        declared_type_params: &HashSet<String>,
    ) -> bool {
        match ty {
            InferType::Struct(name) => {
                declared_type_params.contains(name)
                    && !self.type_table.has_struct(name)
                    && generic_scope.contains(name)
            }
            _ => false,
        }
    }

    fn is_indexable_type(ty: &InferType) -> bool {
        matches!(
            ty,
            InferType::Array(_, _)
                | InferType::Vec(_)
                | InferType::String
                | InferType::Dynamic
                | InferType::Var(_)
        )
    }

    fn is_sliceable_type(ty: &InferType) -> bool {
        matches!(
            ty,
            InferType::Array(_, _)
                | InferType::Vec(_)
                | InferType::String
                | InferType::Dynamic
                | InferType::Var(_)
        )
    }

    fn is_iterable_type(ty: &InferType) -> bool {
        matches!(
            ty,
            InferType::Array(_, _)
                | InferType::Vec(_)
                | InferType::String
                | InferType::Dynamic
                | InferType::Var(_)
        )
    }

    fn is_cast_allowed_resolved(
        &self,
        src: &InferType,
        target: &InferType,
        generic_scope: &HashSet<String>,
        declared_type_params: &HashSet<String>,
    ) -> bool {
        if self.is_active_generic_placeholder_type(src, generic_scope, declared_type_params) {
            return true;
        }

        (src.is_numeric() || *src == InferType::Bool || *src == InferType::Dynamic)
            && (target.is_numeric() || *target == InferType::Bool)
    }

    fn contains_active_generic_placeholder_type(
        &self,
        ty: &InferType,
        generic_scope: &HashSet<String>,
        declared_type_params: &HashSet<String>,
    ) -> bool {
        match ty {
            InferType::Struct(name) => {
                declared_type_params.contains(name)
                    && !self.type_table.has_struct(name)
                    && generic_scope.contains(name)
            }
            InferType::Function { params, ret } => {
                params.iter().any(|p| {
                    self.contains_active_generic_placeholder_type(
                        p,
                        generic_scope,
                        declared_type_params,
                    )
                }) || self.contains_active_generic_placeholder_type(
                    ret,
                    generic_scope,
                    declared_type_params,
                )
            }
            InferType::Array(inner, _) | InferType::Vec(inner) => {
                self.contains_active_generic_placeholder_type(
                    inner,
                    generic_scope,
                    declared_type_params,
                )
            }
            InferType::Tuple(elems) => elems.iter().any(|e| {
                self.contains_active_generic_placeholder_type(e, generic_scope, declared_type_params)
            }),
            _ => false,
        }
    }

    fn validate_return_escapes_in_stmts(
        &mut self,
        stmts: &[TypedStmt],
        fn_return_type: &InferType,
        generic_scope: &HashSet<String>,
        declared_type_params: &HashSet<String>,
    ) {
        for stmt in stmts {
            self.validate_return_escapes_in_stmt(
                stmt,
                fn_return_type,
                generic_scope,
                declared_type_params,
            );
        }
    }

    fn validate_return_escapes_in_stmt(
        &mut self,
        stmt: &TypedStmt,
        fn_return_type: &InferType,
        generic_scope: &HashSet<String>,
        declared_type_params: &HashSet<String>,
    ) {
        match &stmt.kind {
            TypedStmtKind::Return(Some(expr)) => {
                if let Some(name) =
                    self.find_leaked_type_param(&expr.ty, generic_scope, declared_type_params)
                {
                    let return_is_generic_placeholder = self.contains_active_generic_placeholder_type(
                        fn_return_type,
                        generic_scope,
                        declared_type_params,
                    );
                    let is_direct_generic_call = matches!(&expr.kind, TypedExprKind::Call { .. });

                    if !return_is_generic_placeholder && !is_direct_generic_call {
                        self.errors.push(TypeError::member_access(
                            format!(
                                "unresolved generic type parameter '{}' escaped generic context",
                                name
                            ),
                            expr.span,
                        ));
                    }
                }
            }
            TypedStmtKind::Block(stmts) => self.validate_return_escapes_in_stmts(
                stmts,
                fn_return_type,
                generic_scope,
                declared_type_params,
            ),
            TypedStmtKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                self.validate_return_escapes_in_stmt(
                    then_branch,
                    fn_return_type,
                    generic_scope,
                    declared_type_params,
                );
                if let Some(else_branch) = else_branch {
                    self.validate_return_escapes_in_stmt(
                        else_branch,
                        fn_return_type,
                        generic_scope,
                        declared_type_params,
                    );
                }
            }
            TypedStmtKind::While { body, .. }
            | TypedStmtKind::For { body, .. }
            | TypedStmtKind::ForEach { body, .. } => self.validate_return_escapes_in_stmt(
                body,
                fn_return_type,
                generic_scope,
                declared_type_params,
            ),
            TypedStmtKind::Function(_) => {
                // nested functions are validated independently by validate_function
            }
            TypedStmtKind::Expression(_)
            | TypedStmtKind::Let { .. }
            | TypedStmtKind::Return(None)
            | TypedStmtKind::Break
            | TypedStmtKind::Continue
            | TypedStmtKind::Needs(_)
            | TypedStmtKind::StructDecl { .. } => {}
        }
    }
}

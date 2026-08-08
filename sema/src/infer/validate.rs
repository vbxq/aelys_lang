use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::place_spine::{denotes_a_place, spine_is_shared, target_ptr_is_shared};
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
        self.module_globals = stmts
            .iter()
            .filter_map(|s| match &s.kind {
                TypedStmtKind::Let { name, .. } => Some(name.clone()),
                _ => None,
            })
            .collect();
        let generic_scope = HashSet::new();
        for stmt in stmts {
            self.validate_stmt(stmt, &generic_scope, declared_type_params);
        }
    }

    // whole body. over-fencing a global borrow is worse than under-fencing a shadow.
    fn collect_bound_names(stmts: &[TypedStmt], out: &mut HashSet<String>) {
        for stmt in stmts {
            match &stmt.kind {
                TypedStmtKind::Let { name, .. } => {
                    out.insert(name.clone());
                }
                TypedStmtKind::Block(inner) => Self::collect_bound_names(inner, out),
                TypedStmtKind::If {
                    then_branch,
                    else_branch,
                    ..
                } => {
                    Self::collect_bound_names(std::slice::from_ref(then_branch), out);
                    if let Some(e) = else_branch {
                        Self::collect_bound_names(std::slice::from_ref(e), out);
                    }
                }
                TypedStmtKind::While { body, .. } => {
                    Self::collect_bound_names(std::slice::from_ref(body), out)
                }
                TypedStmtKind::For { iterator, body, .. } => {
                    out.insert(iterator.clone());
                    Self::collect_bound_names(std::slice::from_ref(body), out);
                }
                TypedStmtKind::ForEach { iterator, body, .. } => {
                    out.insert(iterator.clone());
                    Self::collect_bound_names(std::slice::from_ref(body), out);
                }
                _ => {}
            }
        }
    }

    fn names_a_global(&self, e: &TypedExpr) -> Option<String> {
        let mut cur = e;
        while let TypedExprKind::Grouping(inner) = &cur.kind {
            cur = inner;
        }
        match &cur.kind {
            TypedExprKind::Identifier(name)
                if self.module_globals.contains(name) && !self.shadowed_globals.contains(name) =>
            {
                Some(name.clone())
            }
            _ => None,
        }
    }

    fn check_write_target(&mut self, target: &TypedExpr, what: &str, span: Span) {
        if !denotes_a_place(target) {
            self.errors
                .push(TypeError::no_place(format!("the target of {what}"), span));
        } else if spine_is_shared(target) {
            self.errors
                .push(TypeError::shared_mut(what.to_string(), span));
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

                if matches!(iterable.ty, InferType::Vec(_)) {
                    self.errors
                        .push(TypeError::vec_foreach_unsupported(iterable.span));
                } else if !Self::is_iterable_type(&iterable.ty)
                    && !self.is_active_generic_placeholder_type(
                        &iterable.ty,
                        generic_scope,
                        declared_type_params,
                    )
                {
                    self.errors.push(TypeError::member_access(
                        format!(
                            "for-each requires an iterable (array or string), got {}",
                            iterable.ty
                        ),
                        iterable.span,
                    ));
                }

                self.validate_stmt(body, generic_scope, declared_type_params);
            }
            TypedStmtKind::Return(Some(expr)) => {
                self.validate_expr(expr, generic_scope, declared_type_params);
                // a reference returned out of a lambda body escapes a
                if self.lambda_depth > 0 && matches!(expr.ty, InferType::Ref { .. }) {
                    self.errors.push(TypeError::closure_ref_unchecked(
                        "a reference is returned",
                        stmt.span,
                    ));
                }
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
            TypedStmtKind::EnumDecl { .. } => {}
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
        let saved_shadow = std::mem::take(&mut self.shadowed_globals);
        // counting them would make e0424 unreachable
        let mut bound: HashSet<String> = func.params.iter().map(|p| p.name.clone()).collect();
        Self::collect_bound_names(&func.body, &mut bound);
        self.shadowed_globals = bound;
        for stmt in &func.body {
            self.validate_stmt(stmt, &generic_scope, declared_type_params);
        }
        self.shadowed_globals = saved_shadow;
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
                if !matches!(callee.kind, TypedExprKind::Identifier(_)) {
                    self.validate_expr(callee, generic_scope, declared_type_params);
                }
                for arg in args {
                    self.validate_expr(arg, generic_scope, declared_type_params);
                }
                self.check_nogc_bound_call(callee, args);
                match &callee.ty {
                    InferType::Function { params, .. } => {
                        if params.len() != args.len() {
                            self.errors.push(TypeError::arity_mismatch(
                                params.len(),
                                args.len(),
                                expr.span,
                                ConstraintReason::Other("function call".to_string()),
                            ));
                        }
                    }
                    InferType::Dynamic => {}
                    other
                        if self.is_active_generic_placeholder_type(
                            other,
                            generic_scope,
                            declared_type_params,
                        ) =>
                    {
                        self.errors.push(TypeError::not_callable(
                            other.clone(),
                            expr.span,
                            ConstraintReason::Other(
                                "function call on unconstrained generic type parameter".to_string(),
                            ),
                        ));
                    }
                    other => {
                        self.errors.push(TypeError::not_callable(
                            other.clone(),
                            expr.span,
                            ConstraintReason::Other("function call".to_string()),
                        ));
                    }
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
                let has_generic_placeholder = self.contains_active_generic_placeholder_type(
                    return_type,
                    generic_scope,
                    declared_type_params,
                ) || params.iter().any(|p| {
                    self.contains_active_generic_placeholder_type(
                        &p.ty,
                        generic_scope,
                        declared_type_params,
                    )
                }) || captures.iter().any(|(_, ty)| {
                    self.contains_active_generic_placeholder_type(
                        ty,
                        generic_scope,
                        declared_type_params,
                    )
                });
                let saved_shadow = std::mem::take(&mut self.shadowed_globals);
                let mut bound: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();
                Self::collect_bound_names(body, &mut bound);
                self.shadowed_globals = bound;
                self.lambda_depth += 1;
                for stmt in body {
                    self.validate_stmt(stmt, generic_scope, declared_type_params);
                }
                self.lambda_depth -= 1;
                self.shadowed_globals = saved_shadow;
                if has_generic_placeholder {
                    self.errors.push(TypeError::member_access(
                        "lambda cannot use active generic type parameters".to_string(),
                        expr.span,
                    ));
                }
            }
            TypedExprKind::Member { object, member } => {
                self.validate_expr(object, generic_scope, declared_type_params);
                self.validate_type(&expr.ty, expr.span, generic_scope, declared_type_params);
                match &object.ty {
                    InferType::String => {
                        if member != "len" {
                            self.errors.push(TypeError::member_access(
                                format!("unknown field '{}' on Str; supported: 'len'", member),
                                expr.span,
                            ));
                        }
                    }
                    InferType::Struct(name) => {
                        if let Some(def) = self.type_table.get_struct(name) {
                            if !def.fields.iter().any(|f| f.name == *member) {
                                self.errors.push(TypeError::member_access(
                                    format!("unknown field '{}' on struct '{}'", member, name),
                                    expr.span,
                                ));
                            }
                        } else if self.is_active_generic_placeholder_type(
                            &object.ty,
                            generic_scope,
                            declared_type_params,
                        ) {
                            self.errors.push(TypeError::member_access(
                                format!(
                                    "field access on unconstrained generic type parameter '{}'",
                                    name
                                ),
                                expr.span,
                            ));
                        } else {
                            self.errors.push(TypeError::member_access(
                                format!("field access on unknown struct type {}", name),
                                expr.span,
                            ));
                        }
                    }
                    InferType::Rc(inner) => {
                        if let InferType::Struct(name) = inner.as_ref() {
                            if let Some(def) = self.type_table.get_struct(name) {
                                if !def.fields.iter().any(|f| f.name == *member) {
                                    self.errors.push(TypeError::member_access(
                                        format!("unknown field '{}' on struct '{}'", member, name),
                                        expr.span,
                                    ));
                                }
                            }
                        } else {
                            self.errors.push(TypeError::member_access(
                                format!(
                                    "field access through `Rc<{inner}>` requires a struct payload"
                                ),
                                expr.span,
                            ));
                        }
                    }
                    InferType::Dynamic => {}
                    other => {
                        self.errors.push(TypeError::member_access(
                            format!("field access on non-struct type {}", other),
                            expr.span,
                        ));
                    }
                }
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
                self.check_write_target(object, "an indexed assignment", expr.span);

                if !Self::is_index_assignable_type(&object.ty)
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
            TypedExprKind::FieldAssign {
                object,
                field: _,
                value,
            } => {
                self.validate_expr(object, generic_scope, declared_type_params);
                self.validate_expr(value, generic_scope, declared_type_params);
                self.check_write_target(object, "a field assignment", expr.span);
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
                if !denotes_a_place(object) {
                    self.errors
                        .push(TypeError::no_place("the base of a slice", expr.span));
                }
                if matches!(object.ty, InferType::Vec(_)) {
                    self.errors
                        .push(TypeError::vec_slice_unsupported(expr.span));
                } else if !Self::is_indexable_type(&object.ty)
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
            TypedExprKind::Reference { mutable, operand } => {
                self.validate_expr(operand, generic_scope, declared_type_params);
                if !denotes_a_place(operand) {
                    self.errors
                        .push(TypeError::no_place("the operand of `&`", expr.span));
                }
                // `&mut *<shared &>` reborrows a shared borrow mutably
                if *mutable && spine_is_shared(operand) {
                    self.errors.push(TypeError::shared_mut(
                        "a `&mut` reborrow of a shared reference",
                        expr.span,
                    ));
                }
                // a global has no borrow-checked local to attach a loan to
                if let Some(name) = self.names_a_global(operand) {
                    self.errors
                        .push(TypeError::global_borrow(name, *mutable, expr.span));
                }
                if self.lambda_depth > 0 {
                    self.errors.push(TypeError::closure_ref_unchecked(
                        "a reference is formed",
                        expr.span,
                    ));
                }
                // a `&` into a call/enum-variant payload field forms no loan and points at a temporary
                if let TypedExprKind::Member { object, .. } = &operand.kind {
                    if matches!(
                        object.kind,
                        TypedExprKind::Call { .. } | TypedExprKind::EnumVariant { .. }
                    ) {
                        self.errors
                            .push(TypeError::payload_field_ref_unsupported(expr.span));
                    }
                }
                if *mutable && Self::ref_operand_has_index_projection(&operand.kind) {
                    self.errors
                        .push(TypeError::mut_index_ref_unsupported(expr.span));
                }
            }
            TypedExprKind::Deref(operand) => {
                self.validate_expr(operand, generic_scope, declared_type_params);
            }
            TypedExprKind::DerefAssign { target, value } => {
                self.validate_expr(target, generic_scope, declared_type_params);
                self.validate_expr(value, generic_scope, declared_type_params);
                // the target of a derefassign is the pointer itself, so the shared test is one
                if target_ptr_is_shared(target) {
                    self.errors.push(TypeError::shared_mut(
                        "an assignment through `*p`",
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

                if !self.is_cast_allowed_resolved(&inner.ty, target) {
                    self.errors.push(TypeError::mismatch(
                        target.clone(),
                        inner.ty.clone(),
                        expr.span,
                        ConstraintReason::InvalidCast,
                    ));
                }
            }
            TypedExprKind::EnumVariant {
                enum_name,
                variant,
                args,
                ..
            } => {
                for arg in args {
                    self.validate_expr(arg, generic_scope, declared_type_params);
                }
                if enum_name == "Vec" && variant == "push" && !args.is_empty() {
                    self.check_write_target(&args[0], "a `Vec::push`", expr.span);
                }
                if args.is_empty() {
                    if let InferType::Enum(name, type_args) = &expr.ty {
                        if let Some(def) = self.type_table.get_enum(name) {
                            if !def.type_params.is_empty()
                                && type_args.len() == def.type_params.len()
                            {
                                let has_unresolved = type_args
                                    .iter()
                                    .any(|a| matches!(a, InferType::Dynamic | InferType::Var(_)));
                                if has_unresolved {
                                    let params_str = def.type_params.join(", ");
                                    self.errors.push(TypeError {
                                    kind: TypeErrorKind::MemberAccess {
                                        message: format!(
                                            "type annotations needed: cannot infer type parameter{} \
                                             <{}> for `{}::{}`",
                                            if def.type_params.len() > 1 { "s" } else { "" },
                                            params_str,
                                            enum_name,
                                            variant,
                                        ),
                                    },
                                    span: expr.span,
                                    reason: ConstraintReason::Other(
                                        "generic enum type parameter inference".to_string(),
                                    ),
                                    secondary_spans: Vec::new(),
                                    help: Some(format!(
                                        "add a type annotation: `let x: {}<...> = {}::{}`",
                                        name, enum_name, variant,
                                    )),
                                    suggestion: None,
                                });
                                }
                            }
                        }
                    }
                }
            }
            TypedExprKind::Block { stmts, tail } => {
                for stmt in stmts {
                    self.validate_stmt(stmt, generic_scope, declared_type_params);
                }
                self.validate_expr(tail, generic_scope, declared_type_params);
            }
            TypedExprKind::Match { scrutinee, arms } => {
                self.validate_expr(scrutinee, generic_scope, declared_type_params);
                for arm in arms {
                    self.validate_expr(&arm.body, generic_scope, declared_type_params);
                }
            }
            // no member node reaches here, so e0304 cannot fire on the intercepted call
            TypedExprKind::ResultAssert { scrutinee, .. } => {
                self.validate_expr(scrutinee, generic_scope, declared_type_params);
            }
            TypedExprKind::Identifier(name) => {
                self.check_nogc_generic_value(name, expr);
            }
            TypedExprKind::Int(_)
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
            InferType::Function { params, ret, .. } => params
                .iter()
                .find_map(|p| self.find_leaked_type_param(p, generic_scope, declared_type_params))
                .or_else(|| self.find_leaked_type_param(ret, generic_scope, declared_type_params)),
            InferType::Array(inner, _) | InferType::Vec(inner) => {
                self.find_leaked_type_param(inner, generic_scope, declared_type_params)
            }
            InferType::Tuple(elems) => elems
                .iter()
                .find_map(|e| self.find_leaked_type_param(e, generic_scope, declared_type_params)),
            InferType::Enum(_, type_args) => type_args
                .iter()
                .find_map(|a| self.find_leaked_type_param(a, generic_scope, declared_type_params)),
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

    // spine is fine, a reborrow is the pointer itself.
    fn ref_operand_has_index_projection(kind: &TypedExprKind) -> bool {
        match kind {
            TypedExprKind::Index { .. } | TypedExprKind::Member { .. } => true,
            TypedExprKind::Deref(inner) => Self::ref_operand_has_index_projection(&inner.kind),
            TypedExprKind::Grouping(inner) => Self::ref_operand_has_index_projection(&inner.kind),
            _ => false,
        }
    }

    fn is_indexable_type(ty: &InferType) -> bool {
        matches!(
            ty,
            InferType::Array(_, _)
                | InferType::Vec(_)
                | InferType::Slice { .. }
                | InferType::String
                | InferType::Dynamic
                | InferType::Var(_)
        )
    }

    fn is_index_assignable_type(ty: &InferType) -> bool {
        matches!(
            ty,
            InferType::Array(_, _)
                | InferType::Vec(_)
                | InferType::Slice { .. }
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

    fn is_cast_allowed_resolved(&self, src: &InferType, target: &InferType) -> bool {
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
            InferType::Function { params, ret, .. } => {
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
            InferType::Array(inner, _) | InferType::Vec(inner) => self
                .contains_active_generic_placeholder_type(
                    inner,
                    generic_scope,
                    declared_type_params,
                ),
            InferType::Tuple(elems) => elems.iter().any(|e| {
                self.contains_active_generic_placeholder_type(
                    e,
                    generic_scope,
                    declared_type_params,
                )
            }),
            InferType::Enum(_, type_args) => type_args.iter().any(|a| {
                self.contains_active_generic_placeholder_type(
                    a,
                    generic_scope,
                    declared_type_params,
                )
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
                    let return_is_generic_placeholder = self
                        .contains_active_generic_placeholder_type(
                            fn_return_type,
                            generic_scope,
                            declared_type_params,
                        );

                    if !return_is_generic_placeholder {
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
            | TypedStmtKind::StructDecl { .. }
            | TypedStmtKind::EnumDecl { .. } => {}
        }
    }
}

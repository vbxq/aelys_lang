
mod block;
mod implicit;
pub(crate) mod let_stmt;
mod loop_stmt;
mod needs;
mod return_stmt;

use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason, TypeError, TypeErrorKind};
use crate::typed_ast::{TypedStmt, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::{ExprKind, Stmt, StmtKind};

impl TypeInference {
    pub(super) fn infer_stmts(&mut self, stmts: &[Stmt]) -> Vec<TypedStmt> {
        let mut out = Vec::with_capacity(stmts.len());
        for s in stmts {
            match &s.kind {
                StmtKind::LetPattern {
                    pattern,
                    type_annotation,
                    initializer,
                } => self.infer_let_pattern(s.span, pattern, type_annotation, initializer, &mut out),
                StmtKind::Expression(_) => {
                    let typed = self.infer_stmt(s);
                    if let TypedStmtKind::Expression(e) = &typed.kind {
                        self.discarded_values.push((e.span, e.ty.clone()));
                    }
                    out.push(typed);
                }
                _ => out.push(self.infer_stmt(s)),
            }
        }
        out
    }

    // a block-like statement without `;` keeps its value, which must be `()` since nothing receives it
    pub(crate) fn report_discarded_values(&mut self, subst: &crate::unify::Substitution) {
        for (span, ty) in std::mem::take(&mut self.discarded_values) {
            let ty = subst.apply(&ty);
            if matches!(
                ty,
                InferType::Null | InferType::Never | InferType::Dynamic | InferType::Var(_)
            ) {
                continue;
            }
            let mut err = TypeError::coded(
                "E0134",
                format!(
                    "this value of type `{ty}` is discarded: a statement gives its value to \
                     nothing, and a statement that starts with a block ends at that block"
                ),
                "value discarded here",
                span,
            );
            err.help = Some("wrap it in parentheses to use it, or end it with `;`".to_string());
            self.errors.push(err);
        }
    }

    fn infer_stmts_into(&mut self, stmts: &[Stmt], out: &mut Vec<TypedStmt>) {
        out.extend(self.infer_stmts(stmts));
    }

    fn infer_let_pattern(
        &mut self,
        span: aelys_syntax::Span,
        pattern: &aelys_syntax::Pattern,
        type_annotation: &Option<aelys_syntax::TypeAnnotation>,
        initializer: &aelys_syntax::Expr,
        out: &mut Vec<TypedStmt>,
    ) {
        self.reject_repeated_bindings(pattern);
        let n = self.try_counter;
        self.try_counter += 1;
        let tmp = format!("__p${n}");
        let kind = self.infer_let_stmt(span, &tmp, false, type_annotation, initializer, false);
        let ty = match &kind {
            TypedStmtKind::Let { var_type, .. } => var_type.clone(),
            _ => InferType::Dynamic,
        };
        out.push(TypedStmt { kind, span });
        let root = aelys_syntax::Expr::new(aelys_syntax::ExprKind::Identifier(tmp), span);
        self.bind_pattern_parts(pattern, &ty, root, out);
    }

    pub(crate) fn reject_repeated_bindings(&mut self, pattern: &aelys_syntax::Pattern) {
        fn walk<'p>(
            pattern: &'p aelys_syntax::Pattern,
            seen: &mut Vec<&'p str>,
            out: &mut Vec<(String, aelys_syntax::Span)>,
        ) {
            use aelys_syntax::Pattern;
            match pattern {
                Pattern::Binding {
                    name, span, sub, ..
                } => {
                    if seen.contains(&name.as_str()) {
                        out.push((name.clone(), *span));
                    } else {
                        seen.push(name);
                    }
                    if let Some(sub) = sub {
                        walk(sub, seen, out);
                    }
                }
                Pattern::Tuple { elems, .. } | Pattern::Variant { fields: elems, .. } => {
                    for e in elems {
                        walk(e, seen, out);
                    }
                }
                Pattern::Or(alts, _) => {
                    if let Some(first) = alts.first() {
                        walk(first, seen, out);
                    }
                }
                Pattern::Struct { fields, .. } => {
                    for (_, e, _) in fields {
                        walk(e, seen, out);
                    }
                }
                Pattern::Lit(..) | Pattern::Range { .. } | Pattern::ConstRange { .. } => {}
                Pattern::Wildcard(_) => {}
            }
        }
        let mut seen = Vec::new();
        let mut repeated = Vec::new();
        walk(pattern, &mut seen, &mut repeated);
        for (name, span) in repeated {
            self.errors.push(TypeError::coded(
                "E0204",
                format!("`{name}` is bound more than once in the same pattern"),
                "bound again here",
                span,
            ));
        }
    }

    fn bind_pattern_parts(
        &mut self,
        pattern: &aelys_syntax::Pattern,
        ty: &InferType,
        source: aelys_syntax::Expr,
        out: &mut Vec<TypedStmt>,
    ) {
        use aelys_syntax::{Expr, ExprKind, Pattern};
        if let Some(converted) = self.resolve_pattern_path(pattern) {
            return self.bind_pattern_parts(&converted, ty, source, out);
        }
        match pattern {
            Pattern::Wildcard(_) => {}
            Pattern::Binding {
                name,
                mutable,
                sub,
                span,
            } => {
                let kind = self.infer_let_stmt(*span, name, *mutable, &None, &source, false);
                out.push(TypedStmt { kind, span: *span });
                if let Some(sub) = sub {
                    self.bind_pattern_parts(sub, ty, source, out);
                }
            }
            Pattern::Tuple { elems, span } => {
                let parts = match ty {
                    InferType::Tuple(parts) => parts.clone(),
                    InferType::Dynamic => {
                        self.bind_refused_pattern(pattern);
                        return;
                    }
                    other => {
                        self.errors.push(TypeError::coded(
                            "E0301",
                            format!("a tuple pattern needs a tuple, found `{other}`"),
                            "not a tuple",
                            *span,
                        ));
                        self.bind_refused_pattern(pattern);
                        return;
                    }
                };
                if parts.len() != elems.len() {
                    self.errors.push(TypeError::coded(
                        "E0301",
                        format!(
                            "the pattern binds {} elements but the tuple `{ty}` has {}",
                            elems.len(),
                            parts.len()
                        ),
                        "wrong number of elements",
                        *span,
                    ));
                    self.bind_refused_pattern(pattern);
                    return;
                }
                for (i, (elem, part)) in elems.iter().zip(parts.iter()).enumerate() {
                    let access = Expr::new(
                        ExprKind::Member {
                            object: Box::new(source.clone()),
                            member: i.to_string(),
                        },
                        *span,
                    );
                    self.bind_pattern_parts(elem, part, access, out);
                }
            }
            Pattern::Struct {
                name,
                fields,
                rest,
                span,
            } => {
                let qualified = self.air_type_name(name, *span);
                if !self.type_table.has_struct(&qualified) {
                    self.refuse_refutable_let(pattern, *span);
                    return;
                }
                let expected = match ty {
                    InferType::Var(_) => {
                        let params = self
                            .type_table
                            .get_struct(&qualified)
                            .map_or(0, |d| d.type_params.len());
                        let args = (0..params).map(|_| self.type_gen.fresh()).collect();
                        let expected = InferType::Struct(qualified.clone(), args);
                        self.constraints.push(Constraint::equal(
                            ty.clone(),
                            expected.clone(),
                            *span,
                            ConstraintReason::Other("a struct pattern".to_string()),
                        ));
                        expected
                    }
                    other => other.clone(),
                };
                let declared = match &expected {
                    InferType::Struct(found, args) if *found == qualified => self
                        .type_table
                        .struct_fields(&qualified, args)
                        .unwrap_or_default(),
                    InferType::Dynamic => {
                        self.bind_refused_pattern(pattern);
                        return;
                    }
                    other => {
                        self.pattern_error(
                            TypeErrorKind::PatternMalformed {
                                detail: format!("pattern `{name} {{ .. }}` matches a `{name}`, but the value has type `{other}`"),
                            },
                            *span,
                        );
                        self.bind_refused_pattern(pattern);
                        return;
                    }
                };
                let unknown = fields
                    .iter()
                    .find(|(f, _, _)| !declared.iter().any(|d| d.name == *f));
                let missing = declared
                    .iter()
                    .find(|d| !fields.iter().any(|(f, _, _)| *f == d.name));
                let detail = match (unknown, missing) {
                    (Some((field, _, _)), _) => {
                        Some(format!("struct `{name}` has no field `{field}`"))
                    }
                    (None, Some(d)) if !rest => Some(format!(
                        "this pattern does not mention field `{}` of `{name}`; name it, or write `..` to skip the fields left out",
                        d.name
                    )),
                    _ => None,
                };
                if let Some(detail) = detail {
                    self.pattern_error(TypeErrorKind::PatternMalformed { detail }, *span);
                    self.bind_refused_pattern(pattern);
                    return;
                }
                for (field, sub, at) in fields {
                    let field_ty = declared
                        .iter()
                        .find(|d| d.name == *field)
                        .map_or(InferType::Dynamic, |d| d.ty.clone());
                    let access = Expr::new(
                        ExprKind::Member {
                            object: Box::new(source.clone()),
                            member: field.clone(),
                        },
                        *at,
                    );
                    self.bind_pattern_parts(sub, &field_ty, access, out);
                }
            }
            Pattern::Variant { span, .. }
            | Pattern::Lit(_, span)
            | Pattern::Range { span, .. }
            | Pattern::ConstRange { span, .. }
            | Pattern::Or(_, span) => self.refuse_refutable_let(pattern, *span),
        }
    }

    fn refuse_refutable_let(&mut self, pattern: &aelys_syntax::Pattern, span: aelys_syntax::Span) {
        let mut err = TypeError::coded(
            "E0101",
            "this pattern can fail to match, so `let` needs an `else`",
            "refutable pattern",
            span,
        );
        err.help = Some("write `let PATTERN = value else { return; };`".to_string());
        self.errors.push(err);
        self.bind_refused_pattern(pattern);
    }

    fn bind_refused_pattern(&mut self, pattern: &aelys_syntax::Pattern) {
        use aelys_syntax::Pattern;
        match pattern {
            Pattern::Binding {
                name,
                mutable,
                sub,
                span,
            } => {
                self.env
                    .define_local_with_span(name.clone(), InferType::Dynamic, *span);
                if *mutable {
                    self.env.mark_mutable(name.clone());
                }
                if let Some(sub) = sub {
                    self.bind_refused_pattern(sub);
                }
            }
            Pattern::Tuple { elems, .. } | Pattern::Variant { fields: elems, .. } => {
                for elem in elems {
                    self.bind_refused_pattern(elem);
                }
            }
            Pattern::Struct { fields, .. } => {
                for (_, field, _) in fields {
                    self.bind_refused_pattern(field);
                }
            }
            Pattern::Or(alternatives, _) => {
                if let Some(first) = alternatives.first() {
                    self.bind_refused_pattern(first);
                }
            }
            Pattern::Wildcard(_)
            | Pattern::Lit(..)
            | Pattern::Range { .. }
            | Pattern::ConstRange { .. } => {}
        }
    }

    pub(super) fn infer_stmt(&mut self, stmt: &Stmt) -> TypedStmt {
        let kind = match &stmt.kind {
            StmtKind::Expression(expr) | StmtKind::Semi(expr) => {
                let typed_expr = self.infer_expr(expr);
                if matches!(stmt.kind, StmtKind::Semi(_)) && matches!(expr.kind, ExprKind::Panic(_))
                {
                    self.constraints.push(Constraint::equal(
                        typed_expr.ty.clone(),
                        InferType::I64,
                        expr.span,
                        ConstraintReason::Other("a panic statement".to_string()),
                    ));
                }
                if let InferType::Enum(name, args) = &typed_expr.ty {
                    if crate::modules::source_type_name(name) == "Result" && args.len() == 2 {
                        self.errors
                            .push(TypeError::must_use(expr.span, args[1].clone()));
                    }
                }
                TypedStmtKind::Expression(typed_expr)
            }
            StmtKind::Discard(expr) => {
                let typed_expr = self.infer_expr(expr);
                TypedStmtKind::Expression(typed_expr)
            }
            StmtKind::Let {
                name,
                mutable,
                type_annotation,
                initializer,
                is_pub,
            } => self.infer_let_stmt(
                stmt.span,
                name,
                *mutable,
                type_annotation,
                initializer,
                *is_pub,
            ),
            StmtKind::Block(stmts) => self.infer_block_stmt(stmts),
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.infer_if_stmt(condition, then_branch, else_branch.as_deref()),
            StmtKind::While { condition, body } => self.infer_while_stmt(condition, body),
            StmtKind::For {
                iterator,
                start,
                end,
                inclusive,
                step,
                body,
            } => self.infer_for_stmt(
                iterator,
                start,
                end,
                *inclusive,
                step.as_ref().as_ref(),
                body,
            ),
            StmtKind::ForEach {
                iterator,
                iterable,
                body,
            } => self.infer_for_each_stmt(iterator, iterable, body, stmt.span),
            StmtKind::Return(expr) => self.infer_return_stmt(stmt.span, expr.as_ref()),
            StmtKind::Break(value) => {
                let typed = value.as_ref().map(|v| self.infer_expr(v));
                if let Some(Some(breaks)) = self.loop_breaks.last_mut() {
                    breaks.push(typed.as_ref().map_or(InferType::Null, |t| t.ty.clone()));
                }
                TypedStmtKind::Break(typed)
            }
            StmtKind::Continue => TypedStmtKind::Continue,
            StmtKind::Function(func) => {
                let typed_func = self.infer_function(func);
                TypedStmtKind::Function(typed_func)
            }
            StmtKind::Needs(needs) => {
                self.handle_needs_stmt(needs);
                TypedStmtKind::Needs(needs.clone())
            }
            StmtKind::StructDecl {
                name,
                type_params,
                fields,
                is_pub,
            } => {
                let saved = std::mem::replace(&mut self.type_params_in_scope, type_params.clone());
                let typed_fields = fields
                    .iter()
                    .map(|f| {
                        let ty = self.type_from_annotation(&f.type_annotation);
                        (f.name.clone(), ty)
                    })
                    .collect();
                self.type_params_in_scope = saved;
                TypedStmtKind::StructDecl {
                    name: name.clone(),
                    type_params: type_params.clone(),
                    fields: typed_fields,
                    is_pub: *is_pub,
                }
            }
            StmtKind::EnumDecl {
                name,
                type_params,
                variants,
                is_pub,
                ..
            } => {
                let enum_def = self.type_table.get_enum(name).cloned();
                TypedStmtKind::EnumDecl {
                    name: name.clone(),
                    type_params: type_params.clone(),
                    variants: variants
                        .iter()
                        .enumerate()
                        .map(|(i, v)| {
                            let data = enum_def
                                .as_ref()
                                .and_then(|def| def.variants.iter().find(|ev| ev.name == v.name))
                                .map(|ev| ev.data.clone())
                                .unwrap_or_default();
                            (v.name.clone(), i as u32, data)
                        })
                        .collect(),
                    is_pub: *is_pub,
                }
            }
            StmtKind::LetPattern { .. } => {
                let mut parts = Vec::new();
                self.infer_stmts_into(std::slice::from_ref(stmt), &mut parts);
                TypedStmtKind::Block(parts)
            }
            StmtKind::Const { .. } | StmtKind::TypeAlias { .. } => {
                let what = if matches!(stmt.kind, StmtKind::Const { .. }) {
                    "a const"
                } else {
                    "a type alias"
                };
                self.errors.push(TypeError::coded(
                    "E0101",
                    format!("{what} is declared at module level"),
                    "not at module level",
                    stmt.span,
                ));
                TypedStmtKind::Block(Vec::new())
            }
            StmtKind::Trait(decl) => {
                self.errors.push(TypeError::coded(
                    "E0757",
                    "a `trait` belongs at the top level of a module".to_string(),
                    "nested trait",
                    decl.span,
                ));
                TypedStmtKind::Block(Vec::new())
            }
            StmtKind::Impl(decl) => {
                self.errors.push(TypeError::coded(
                    "E0340",
                    "an `impl` block belongs at the top level of a module".to_string(),
                    "nested impl",
                    decl.span,
                ));
                TypedStmtKind::Block(Vec::new())
            }
        };

        TypedStmt {
            kind,
            span: stmt.span,
        }
    }
}

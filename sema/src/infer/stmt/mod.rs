
mod block;
mod implicit;
pub(crate) mod let_stmt;
mod loop_stmt;
mod needs;
mod return_stmt;

use super::TypeInference;
use crate::constraint::TypeError;
use crate::typed_ast::{TypedStmt, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::{Stmt, StmtKind};

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
                _ => out.push(self.infer_stmt(s)),
            }
        }
        out
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
                Pattern::Lit(..) | Pattern::Range { .. } => {}
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
            Pattern::Variant { span, .. }
            | Pattern::Lit(_, span)
            | Pattern::Range { span, .. }
            | Pattern::Struct { span, .. }
            | Pattern::Or(_, span) => {
                self.errors.push(TypeError::coded(
                    "E0101",
                    "a let pattern cannot fail; use match",
                    "refutable pattern",
                    *span,
                ));
            }
        }
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
            Pattern::Wildcard(_) | Pattern::Lit(..) | Pattern::Range { .. } => {}
        }
    }

    pub(super) fn infer_stmt(&mut self, stmt: &Stmt) -> TypedStmt {
        let kind = match &stmt.kind {
            StmtKind::Expression(expr) => {
                let typed_expr = self.infer_expr(expr);
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
            StmtKind::Break => TypedStmtKind::Break,
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

use super::TypeInference;
use crate::typed_ast::{TypedExpr, TypedExprKind, TypedParam, TypedStmt, TypedStmtKind};
use crate::types::InferType;
use std::collections::HashSet;

impl TypeInference {
    /// Collect captures from a list of statements
    pub(super) fn collect_captures_from_stmts(
        &self,
        stmts: &[TypedStmt],
        params: &[TypedParam],
    ) -> Vec<(String, InferType)> {
        let mut captures = Vec::new();
        let mut seen = HashSet::new();

        let mut local_names: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();

        for stmt in stmts {
            self.collect_captures_from_stmt(stmt, &mut local_names, &mut captures, &mut seen);
        }

        captures
    }

    fn collect_captures_from_stmt(
        &self,
        stmt: &TypedStmt,
        local_names: &mut HashSet<String>,
        captures: &mut Vec<(String, InferType)>,
        seen: &mut HashSet<String>,
    ) {
        match &stmt.kind {
            TypedStmtKind::Expression(expr) => {
                self.collect_captures_inner(expr, local_names, captures, seen);
            }
            TypedStmtKind::Let {
                name, initializer, ..
            } => {
                // we process the initializer before adding the name, so that `let x = x + 1` captures `x` from the outer scope
                self.collect_captures_inner(initializer, local_names, captures, seen);
                // add the let-bound name so subsequent uses don't get captured
                local_names.insert(name.clone());
            }
            TypedStmtKind::Block(stmts) => {
                for s in stmts {
                    self.collect_captures_from_stmt(s, local_names, captures, seen);
                }
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.collect_captures_inner(condition, local_names, captures, seen);
                self.collect_captures_from_stmt(then_branch, local_names, captures, seen);
                if let Some(els) = else_branch {
                    self.collect_captures_from_stmt(els, local_names, captures, seen);
                }
            }
            TypedStmtKind::While { condition, body } => {
                self.collect_captures_inner(condition, local_names, captures, seen);
                self.collect_captures_from_stmt(body, local_names, captures, seen);
            }
            TypedStmtKind::For {
                iterator,
                start,
                end,
                step,
                body,
                ..
            } => {
                self.collect_captures_inner(start, local_names, captures, seen);
                self.collect_captures_inner(end, local_names, captures, seen);
                if let Some(step_expr) = step.as_ref().as_ref() {
                    self.collect_captures_inner(step_expr, local_names, captures, seen);
                }
                // The for-loop iterator is a local binding.
                local_names.insert(iterator.clone());
                self.collect_captures_from_stmt(body, local_names, captures, seen);
            }
            TypedStmtKind::ForEach {
                iterator,
                iterable,
                body,
                ..
            } => {
                self.collect_captures_inner(iterable, local_names, captures, seen);
                // The for-each iterator is a local binding.
                local_names.insert(iterator.clone());
                self.collect_captures_from_stmt(body, local_names, captures, seen);
            }
            TypedStmtKind::Return(Some(expr)) => {
                self.collect_captures_inner(expr, local_names, captures, seen);
            }
            TypedStmtKind::Return(None) | TypedStmtKind::Break | TypedStmtKind::Continue => {}
            TypedStmtKind::Function(_) => {}
            TypedStmtKind::Needs(_) => {}
            TypedStmtKind::StructDecl { .. } => {}
        }
    }

    fn collect_captures_inner(
        &self,
        expr: &TypedExpr,
        locals: &HashSet<String>,
        captures: &mut Vec<(String, InferType)>,
        seen: &mut HashSet<String>,
    ) {
        match &expr.kind {
            TypedExprKind::Identifier(name) => {
                if !locals.contains(name)
                    && !seen.contains(name)
                    && let Some(ty) = self.env.captures().get(name)
                {
                    captures.push((name.clone(), ty.clone()));
                    seen.insert(name.clone());
                }
            }
            TypedExprKind::Binary { left, right, .. } => {
                self.collect_captures_inner(left, locals, captures, seen);
                self.collect_captures_inner(right, locals, captures, seen);
            }
            TypedExprKind::Unary { operand, .. } => {
                self.collect_captures_inner(operand, locals, captures, seen);
            }
            TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
                self.collect_captures_inner(left, locals, captures, seen);
                self.collect_captures_inner(right, locals, captures, seen);
            }
            TypedExprKind::Call { callee, args } => {
                self.collect_captures_inner(callee, locals, captures, seen);
                for arg in args {
                    self.collect_captures_inner(arg, locals, captures, seen);
                }
            }
            TypedExprKind::Assign { value, .. } => {
                self.collect_captures_inner(value, locals, captures, seen);
            }
            TypedExprKind::Grouping(inner) => {
                self.collect_captures_inner(inner, locals, captures, seen);
            }
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.collect_captures_inner(condition, locals, captures, seen);
                self.collect_captures_inner(then_branch, locals, captures, seen);
                self.collect_captures_inner(else_branch, locals, captures, seen);
            }
            TypedExprKind::Lambda(inner) => {
                self.collect_captures_inner(inner, locals, captures, seen);
            }
            TypedExprKind::LambdaInner {
                params: inner_params,
                body: stmts,
                ..
            } => {
                // Build a NEW local names set for the inner lambda using its
                // own parameters, not the outer lambda's locals/params.
                let mut inner_locals: HashSet<String> =
                    inner_params.iter().map(|p| p.name.clone()).collect();
                for stmt in stmts {
                    self.collect_captures_from_stmt(stmt, &mut inner_locals, captures, seen);
                }
            }
            TypedExprKind::Member { object, .. } => {
                self.collect_captures_inner(object, locals, captures, seen);
            }
            TypedExprKind::ArrayLiteral { elements, .. }
            | TypedExprKind::VecLiteral { elements, .. } => {
                for elem in elements {
                    self.collect_captures_inner(elem, locals, captures, seen);
                }
            }
            TypedExprKind::ArraySized {
                size, fill_value, ..
            } => {
                self.collect_captures_inner(size, locals, captures, seen);
                if let Some(fv) = fill_value {
                    self.collect_captures_inner(fv, locals, captures, seen);
                }
            }
            TypedExprKind::Index { object, index } => {
                self.collect_captures_inner(object, locals, captures, seen);
                self.collect_captures_inner(index, locals, captures, seen);
            }
            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
                self.collect_captures_inner(object, locals, captures, seen);
                self.collect_captures_inner(index, locals, captures, seen);
                self.collect_captures_inner(value, locals, captures, seen);
            }
            TypedExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.collect_captures_inner(s, locals, captures, seen);
                }
                if let Some(e) = end {
                    self.collect_captures_inner(e, locals, captures, seen);
                }
            }
            TypedExprKind::Slice { object, range } => {
                self.collect_captures_inner(object, locals, captures, seen);
                self.collect_captures_inner(range, locals, captures, seen);
            }
            TypedExprKind::FmtString(parts) => {
                for part in parts {
                    if let crate::typed_ast::TypedFmtStringPart::Expr(e) = part {
                        self.collect_captures_inner(e, locals, captures, seen);
                    }
                }
            }
            TypedExprKind::StructLiteral { fields, .. } => {
                for (_, value) in fields {
                    self.collect_captures_inner(value, locals, captures, seen);
                }
            }
            TypedExprKind::Cast { expr, .. } => {
                self.collect_captures_inner(expr, locals, captures, seen);
            }
            TypedExprKind::Int(_)
            | TypedExprKind::Float(_)
            | TypedExprKind::Bool(_)
            | TypedExprKind::String(_)
            | TypedExprKind::Null => {}
        }
    }
}

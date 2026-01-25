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

        let param_names: HashSet<String> = params.iter().map(|p| p.name.clone()).collect();

        for stmt in stmts {
            self.collect_captures_from_stmt(stmt, &param_names, &mut captures, &mut seen);
        }

        captures
    }

    fn collect_captures_from_stmt(
        &self,
        stmt: &TypedStmt,
        param_names: &HashSet<String>,
        captures: &mut Vec<(String, InferType)>,
        seen: &mut HashSet<String>,
    ) {
        match &stmt.kind {
            TypedStmtKind::Expression(expr) => {
                self.collect_captures_inner(expr, param_names, captures, seen);
            }
            TypedStmtKind::Let { initializer, .. } => {
                self.collect_captures_inner(initializer, param_names, captures, seen);
            }
            TypedStmtKind::Block(stmts) => {
                for s in stmts {
                    self.collect_captures_from_stmt(s, param_names, captures, seen);
                }
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.collect_captures_inner(condition, param_names, captures, seen);
                self.collect_captures_from_stmt(then_branch, param_names, captures, seen);
                if let Some(els) = else_branch {
                    self.collect_captures_from_stmt(els, param_names, captures, seen);
                }
            }
            TypedStmtKind::While { condition, body } => {
                self.collect_captures_inner(condition, param_names, captures, seen);
                self.collect_captures_from_stmt(body, param_names, captures, seen);
            }
            TypedStmtKind::For {
                start,
                end,
                step,
                body,
                ..
            } => {
                self.collect_captures_inner(start, param_names, captures, seen);
                self.collect_captures_inner(end, param_names, captures, seen);
                if let Some(step_expr) = step {
                    self.collect_captures_inner(step_expr, param_names, captures, seen);
                }
                self.collect_captures_from_stmt(body, param_names, captures, seen);
            }
            TypedStmtKind::Return(Some(expr)) => {
                self.collect_captures_inner(expr, param_names, captures, seen);
            }
            TypedStmtKind::Return(None) | TypedStmtKind::Break | TypedStmtKind::Continue => {}
            TypedStmtKind::Function(_) => {}
            TypedStmtKind::Needs(_) => {}
        }
    }

    fn collect_captures_inner(
        &self,
        expr: &TypedExpr,
        params: &HashSet<String>,
        captures: &mut Vec<(String, InferType)>,
        seen: &mut HashSet<String>,
    ) {
        match &expr.kind {
            TypedExprKind::Identifier(name) => {
                if !params.contains(name) && !seen.contains(name) {
                    if let Some(ty) = self.env.captures().get(name) {
                        captures.push((name.clone(), ty.clone()));
                        seen.insert(name.clone());
                    }
                }
            }
            TypedExprKind::Binary { left, right, .. } => {
                self.collect_captures_inner(left, params, captures, seen);
                self.collect_captures_inner(right, params, captures, seen);
            }
            TypedExprKind::Unary { operand, .. } => {
                self.collect_captures_inner(operand, params, captures, seen);
            }
            TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
                self.collect_captures_inner(left, params, captures, seen);
                self.collect_captures_inner(right, params, captures, seen);
            }
            TypedExprKind::Call { callee, args } => {
                self.collect_captures_inner(callee, params, captures, seen);
                for arg in args {
                    self.collect_captures_inner(arg, params, captures, seen);
                }
            }
            TypedExprKind::Assign { value, .. } => {
                self.collect_captures_inner(value, params, captures, seen);
            }
            TypedExprKind::Grouping(inner) => {
                self.collect_captures_inner(inner, params, captures, seen);
            }
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.collect_captures_inner(condition, params, captures, seen);
                self.collect_captures_inner(then_branch, params, captures, seen);
                self.collect_captures_inner(else_branch, params, captures, seen);
            }
            TypedExprKind::Lambda(inner) => {
                self.collect_captures_inner(inner, params, captures, seen);
            }
            TypedExprKind::LambdaInner { body: stmts, .. } => {
                for stmt in stmts {
                    self.collect_captures_from_stmt(stmt, params, captures, seen);
                }
            }
            TypedExprKind::Member { object, .. } => {
                self.collect_captures_inner(object, params, captures, seen);
            }
            TypedExprKind::Int(_)
            | TypedExprKind::Float(_)
            | TypedExprKind::Bool(_)
            | TypedExprKind::String(_)
            | TypedExprKind::Null => {}
        }
    }
}

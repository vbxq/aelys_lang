// collects all variable reads in the program

use aelys_sema::{TypedExpr, TypedExprKind, TypedFunction, TypedStmt, TypedStmtKind};
use std::collections::HashSet;

pub fn collect_used_vars(stmts: &[TypedStmt]) -> HashSet<String> {
    let mut used = HashSet::new();
    for stmt in stmts { collect_uses_in_stmt(stmt, &mut used); }
    used
}

pub fn collect_uses_in_stmt_recursive(stmt: &TypedStmt, used: &mut HashSet<String>) {
    collect_uses_in_stmt(stmt, used);
}

fn collect_uses_in_stmt(stmt: &TypedStmt, used: &mut HashSet<String>) {
    match &stmt.kind {
        TypedStmtKind::Expression(expr) => collect_uses_in_expr(expr, used),
        TypedStmtKind::Let { initializer, .. } => {
            // initializer might use other vars, but name itself is a def not a use
            collect_uses_in_expr(initializer, used);
        }
        TypedStmtKind::Block(stmts) => {
            for s in stmts { collect_uses_in_stmt(s, used); }
        }
        TypedStmtKind::If { condition, then_branch, else_branch } => {
            collect_uses_in_expr(condition, used);
            collect_uses_in_stmt(then_branch, used);
            if let Some(else_b) = else_branch { collect_uses_in_stmt(else_b, used); }
        }
        TypedStmtKind::While { condition, body } => {
            collect_uses_in_expr(condition, used);
            collect_uses_in_stmt(body, used);
        }
        TypedStmtKind::For { iterator, start, end, step, body, .. } => {
            collect_uses_in_expr(start, used);
            collect_uses_in_expr(end, used);
            if let Some(s) = step { collect_uses_in_expr(s, used); }
            collect_uses_in_stmt(body, used);
            used.insert(iterator.clone()); // always mark iterator as used (conservative)
        }
        TypedStmtKind::Return(Some(expr)) => collect_uses_in_expr(expr, used),
        TypedStmtKind::Function(func) => collect_uses_in_function(func, used),
        TypedStmtKind::Return(None) | TypedStmtKind::Break
        | TypedStmtKind::Continue | TypedStmtKind::Needs(_) => {}
    }
}

fn collect_uses_in_function(func: &TypedFunction, used: &mut HashSet<String>) {
    for (name, _) in &func.captures { used.insert(name.clone()); }
    for stmt in &func.body { collect_uses_in_stmt(stmt, used); }
}

fn collect_uses_in_expr(expr: &TypedExpr, used: &mut HashSet<String>) {
    match &expr.kind {
        TypedExprKind::Identifier(name) => { used.insert(name.clone()); }
        TypedExprKind::Binary { left, right, .. } => {
            collect_uses_in_expr(left, used);
            collect_uses_in_expr(right, used);
        }
        TypedExprKind::Unary { operand, .. } => collect_uses_in_expr(operand, used),
        TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
            collect_uses_in_expr(left, used);
            collect_uses_in_expr(right, used);
        }
        TypedExprKind::Call { callee, args } => {
            collect_uses_in_expr(callee, used);
            for arg in args { collect_uses_in_expr(arg, used); }
        }
        TypedExprKind::Assign { name, value } => {
            used.insert(name.clone()); // assignment is a use (var must exist)
            collect_uses_in_expr(value, used);
        }
        TypedExprKind::Grouping(inner) => collect_uses_in_expr(inner, used),
        TypedExprKind::If { condition, then_branch, else_branch } => {
            collect_uses_in_expr(condition, used);
            collect_uses_in_expr(then_branch, used);
            collect_uses_in_expr(else_branch, used);
        }
        TypedExprKind::Lambda(inner) => collect_uses_in_expr(inner, used),
        TypedExprKind::LambdaInner { captures, body, .. } => {
            for (name, _) in captures { used.insert(name.clone()); }
            for stmt in body { collect_uses_in_stmt(stmt, used); }
        }
        TypedExprKind::Member { object, .. } => collect_uses_in_expr(object, used),
        TypedExprKind::ArrayLiteral { elements, .. } | TypedExprKind::VecLiteral { elements, .. } => {
            for elem in elements { collect_uses_in_expr(elem, used); }
        }
        TypedExprKind::ArraySized { size, .. } => {
            collect_uses_in_expr(size, used);
        }
        TypedExprKind::Index { object, index } => {
            collect_uses_in_expr(object, used);
            collect_uses_in_expr(index, used);
        }
        TypedExprKind::IndexAssign { object, index, value } => {
            collect_uses_in_expr(object, used);
            collect_uses_in_expr(index, used);
            collect_uses_in_expr(value, used);
        }
        TypedExprKind::Range { start, end, .. } => {
            if let Some(s) = start { collect_uses_in_expr(s, used); }
            if let Some(e) = end { collect_uses_in_expr(e, used); }
        }
        TypedExprKind::Slice { object, range } => {
            collect_uses_in_expr(object, used);
            collect_uses_in_expr(range, used);
        }
        TypedExprKind::FmtString(parts) => {
            for part in parts {
                if let aelys_sema::TypedFmtStringPart::Expr(e) = part {
                    collect_uses_in_expr(e, used);
                }
            }
        }
        TypedExprKind::Int(_) | TypedExprKind::Float(_) | TypedExprKind::Bool(_)
        | TypedExprKind::String(_) | TypedExprKind::Null => {}
    }
}

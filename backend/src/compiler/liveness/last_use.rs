use super::LivenessAnalysis;
use aelys_sema::{TypedExpr, TypedExprKind, TypedStmt, TypedStmtKind};
use std::collections::HashSet;

pub(super) fn compute_last_use_points(analysis: &mut LivenessAnalysis, stmts: &[TypedStmt]) {
    for (idx, stmt) in stmts.iter().enumerate() {
        let mut uses = HashSet::new();
        collect_all_uses_in_stmt(stmt, &mut uses);

        for var in uses {
            if !analysis.captured_vars.contains(&var) {
                analysis.last_use_point.insert(var, idx);
            }
        }
    }
}

fn collect_all_uses_in_stmt(stmt: &TypedStmt, uses: &mut HashSet<String>) {
    match &stmt.kind {
        TypedStmtKind::Expression(expr) => {
            collect_all_uses_in_expr(expr, uses);
        }
        TypedStmtKind::Let { initializer, .. } => {
            collect_all_uses_in_expr(initializer, uses);
        }
        TypedStmtKind::Block(stmts) => {
            for s in stmts {
                collect_all_uses_in_stmt(s, uses);
            }
        }
        TypedStmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_all_uses_in_expr(condition, uses);
            collect_all_uses_in_stmt(then_branch, uses);
            if let Some(else_b) = else_branch {
                collect_all_uses_in_stmt(else_b, uses);
            }
        }
        TypedStmtKind::While { condition, body } => {
            collect_all_uses_in_expr(condition, uses);
            collect_all_uses_in_stmt(body, uses);
        }
        TypedStmtKind::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            collect_all_uses_in_expr(start, uses);
            collect_all_uses_in_expr(end, uses);
            if let Some(s) = &**step {
                collect_all_uses_in_expr(s, uses);
            }
            collect_all_uses_in_stmt(body, uses);
        }
        TypedStmtKind::Return(Some(expr)) => {
            collect_all_uses_in_expr(expr, uses);
        }
        TypedStmtKind::Function(inner_func) => {
            for (name, _) in &inner_func.captures {
                uses.insert(name.clone());
            }
        }
        TypedStmtKind::ForEach { iterable, body, .. } => {
            collect_all_uses_in_expr(iterable, uses);
            collect_all_uses_in_stmt(body, uses);
        }
        TypedStmtKind::Return(None)
        | TypedStmtKind::Break
        | TypedStmtKind::Continue
        | TypedStmtKind::Needs(_)
        | TypedStmtKind::StructDecl { .. } => {}
    }
}

fn collect_all_uses_in_expr(expr: &TypedExpr, uses: &mut HashSet<String>) {
    match &expr.kind {
        TypedExprKind::Identifier(name) => {
            uses.insert(name.clone());
        }
        TypedExprKind::Binary { left, right, .. } => {
            collect_all_uses_in_expr(left, uses);
            collect_all_uses_in_expr(right, uses);
        }
        TypedExprKind::Unary { operand, .. } => {
            collect_all_uses_in_expr(operand, uses);
        }
        TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
            collect_all_uses_in_expr(left, uses);
            collect_all_uses_in_expr(right, uses);
        }
        TypedExprKind::Call { callee, args } => {
            collect_all_uses_in_expr(callee, uses);
            for arg in args {
                collect_all_uses_in_expr(arg, uses);
            }
        }
        TypedExprKind::Assign { name, value } => {
            uses.insert(name.clone());
            collect_all_uses_in_expr(value, uses);
        }
        TypedExprKind::Grouping(inner) => {
            collect_all_uses_in_expr(inner, uses);
        }
        TypedExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_all_uses_in_expr(condition, uses);
            collect_all_uses_in_expr(then_branch, uses);
            collect_all_uses_in_expr(else_branch, uses);
        }
        TypedExprKind::Lambda(inner) => {
            collect_all_uses_in_expr(inner, uses);
        }
        TypedExprKind::LambdaInner { captures, body, .. } => {
            for (name, _) in captures {
                uses.insert(name.clone());
            }
            for s in body {
                collect_all_uses_in_stmt(s, uses);
            }
        }
        TypedExprKind::Member { object, .. } => {
            collect_all_uses_in_expr(object, uses);
        }
        TypedExprKind::ArrayLiteral { elements, .. }
        | TypedExprKind::VecLiteral { elements, .. } => {
            for elem in elements {
                collect_all_uses_in_expr(elem, uses);
            }
        }
        TypedExprKind::ArraySized { size, .. } => {
            collect_all_uses_in_expr(size, uses);
        }
        TypedExprKind::Index { object, index } => {
            collect_all_uses_in_expr(object, uses);
            collect_all_uses_in_expr(index, uses);
        }
        TypedExprKind::IndexAssign {
            object,
            index,
            value,
        } => {
            collect_all_uses_in_expr(object, uses);
            collect_all_uses_in_expr(index, uses);
            collect_all_uses_in_expr(value, uses);
        }
        TypedExprKind::Range { start, end, .. } => {
            if let Some(s) = start {
                collect_all_uses_in_expr(s, uses);
            }
            if let Some(e) = end {
                collect_all_uses_in_expr(e, uses);
            }
        }
        TypedExprKind::Slice { object, range } => {
            collect_all_uses_in_expr(object, uses);
            collect_all_uses_in_expr(range, uses);
        }
        TypedExprKind::FmtString(parts) => {
            for part in parts {
                if let aelys_sema::TypedFmtStringPart::Expr(e) = part {
                    collect_all_uses_in_expr(e, uses);
                }
            }
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                collect_all_uses_in_expr(value, uses);
            }
        }
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Null => {}
    }
}

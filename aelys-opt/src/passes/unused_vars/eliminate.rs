// removes unused let bindings (unless they have side effects)

use super::super::OptimizationStats;
use aelys_sema::{TypedExpr, TypedExprKind, TypedFunction, TypedStmt, TypedStmtKind};
use std::collections::HashSet;

pub fn eliminate_unused_in_block(
    stmts: &mut Vec<TypedStmt>,
    used_vars: &HashSet<String>,
    stats: &mut OptimizationStats,
) {
    for stmt in stmts.iter_mut() {
        eliminate_unused_in_stmt(stmt, used_vars, stats);
    }

    stmts.retain(|stmt| {
        if let TypedStmtKind::Let {
            name,
            initializer,
            is_pub,
            ..
        } = &stmt.kind
        {
            if *is_pub {
                return true;
            } // keep pub exports
            if !used_vars.contains(name) && !has_side_effects(initializer) {
                stats.dead_code_eliminated += 1;
                return false;
            }
        }
        true
    });
}

fn eliminate_unused_in_stmt(
    stmt: &mut TypedStmt,
    used_vars: &HashSet<String>,
    stats: &mut OptimizationStats,
) {
    match &mut stmt.kind {
        TypedStmtKind::Block(stmts) => eliminate_unused_in_block(stmts, used_vars, stats),
        TypedStmtKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            eliminate_unused_in_stmt(then_branch, used_vars, stats);
            if let Some(else_b) = else_branch {
                eliminate_unused_in_stmt(else_b, used_vars, stats);
            }
        }
        TypedStmtKind::While { body, .. } | TypedStmtKind::For { body, .. } => {
            eliminate_unused_in_stmt(body, used_vars, stats);
        }
        TypedStmtKind::Function(func) => eliminate_unused_in_function(func, used_vars, stats),
        _ => {}
    }
}

fn eliminate_unused_in_function(
    func: &mut TypedFunction,
    used_vars: &HashSet<String>,
    stats: &mut OptimizationStats,
) {
    let mut local_used = used_vars.clone();
    for stmt in &func.body {
        super::analysis::collect_uses_in_stmt_recursive(stmt, &mut local_used);
    }
    for param in &func.params {
        local_used.insert(param.name.clone());
    }
    eliminate_unused_in_block(&mut func.body, &local_used, stats);
}

fn has_side_effects(expr: &TypedExpr) -> bool {
    match &expr.kind {
        TypedExprKind::Call { .. } | TypedExprKind::Assign { .. } => true,
        TypedExprKind::Binary { left, right, .. } => {
            has_side_effects(left) || has_side_effects(right)
        }
        TypedExprKind::Unary { operand, .. } => has_side_effects(operand),
        TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
            has_side_effects(left) || has_side_effects(right)
        }
        TypedExprKind::Grouping(inner) => has_side_effects(inner),
        TypedExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            has_side_effects(condition)
                || has_side_effects(then_branch)
                || has_side_effects(else_branch)
        }
        TypedExprKind::Lambda(_) | TypedExprKind::LambdaInner { .. } => false,
        TypedExprKind::Member { object, .. } => has_side_effects(object),
        TypedExprKind::ArrayLiteral { elements, .. }
        | TypedExprKind::VecLiteral { elements, .. } => elements.iter().any(has_side_effects),
        TypedExprKind::ArraySized { size, .. } => has_side_effects(size),
        TypedExprKind::Index { object, index } => {
            has_side_effects(object) || has_side_effects(index)
        }
        TypedExprKind::IndexAssign { .. } => true, // assignment has side effects
        TypedExprKind::Range { start, end, .. } => {
            start.as_ref().is_some_and(|s| has_side_effects(s))
                || end.as_ref().is_some_and(|e| has_side_effects(e))
        }
        TypedExprKind::Slice { object, range } => {
            has_side_effects(object) || has_side_effects(range)
        }
        TypedExprKind::FmtString(parts) => parts.iter().any(|p| {
            if let aelys_sema::TypedFmtStringPart::Expr(e) = p {
                has_side_effects(e)
            } else {
                false
            }
        }),
        TypedExprKind::Identifier(_)
        | TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Null => false,
    }
}

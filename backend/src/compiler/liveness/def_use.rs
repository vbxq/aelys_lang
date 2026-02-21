use super::LivenessAnalysis;
use aelys_sema::{TypedExpr, TypedExprKind, TypedStmt, TypedStmtKind};
use std::collections::HashSet;

pub(super) fn compute_def_use(analysis: &mut LivenessAnalysis, stmts: &[TypedStmt]) {
    for (idx, stmt) in stmts.iter().enumerate() {
        let mut defs = HashSet::new();
        let mut uses = HashSet::new();

        collect_def_use_stmt(analysis, stmt, &mut defs, &mut uses);

        analysis.def.insert(idx, defs);
        analysis.use_set.insert(idx, uses);
    }
}

fn collect_def_use_stmt(
    analysis: &mut LivenessAnalysis,
    stmt: &TypedStmt,
    defs: &mut HashSet<String>,
    uses: &mut HashSet<String>,
) {
    match &stmt.kind {
        TypedStmtKind::Expression(expr) => {
            collect_uses_expr(analysis, expr, uses);
        }
        TypedStmtKind::Let {
            name, initializer, ..
        } => {
            collect_uses_expr(analysis, initializer, uses);
            defs.insert(name.clone());
        }
        TypedStmtKind::Block(stmts) => {
            for s in stmts {
                collect_def_use_stmt(analysis, s, defs, uses);
            }
        }
        TypedStmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_uses_expr(analysis, condition, uses);
            collect_def_use_stmt(analysis, then_branch, defs, uses);
            if let Some(else_b) = else_branch {
                collect_def_use_stmt(analysis, else_b, defs, uses);
            }
        }
        TypedStmtKind::While { condition, body } => {
            collect_uses_expr(analysis, condition, uses);
            collect_def_use_stmt(analysis, body, defs, uses);
        }
        TypedStmtKind::For {
            iterator,
            start,
            end,
            step,
            body,
            ..
        } => {
            collect_uses_expr(analysis, start, uses);
            collect_uses_expr(analysis, end, uses);
            if let Some(s) = &**step {
                collect_uses_expr(analysis, s, uses);
            }
            defs.insert(iterator.clone());
            collect_def_use_stmt(analysis, body, defs, uses);
        }
        TypedStmtKind::Return(Some(expr)) => {
            collect_uses_expr(analysis, expr, uses);
        }
        TypedStmtKind::Function(inner_func) => {
            for (name, _) in &inner_func.captures {
                uses.insert(name.clone());
                analysis.captured_vars.insert(name.clone());
            }
        }
        TypedStmtKind::ForEach {
            iterator,
            iterable,
            body,
            ..
        } => {
            collect_uses_expr(analysis, iterable, uses);
            defs.insert(iterator.clone());
            collect_def_use_stmt(analysis, body, defs, uses);
        }
        TypedStmtKind::Return(None)
        | TypedStmtKind::Break
        | TypedStmtKind::Continue
        | TypedStmtKind::Needs(_)
        | TypedStmtKind::StructDecl { .. } => {}
    }
}

fn collect_uses_expr(
    analysis: &mut LivenessAnalysis,
    expr: &TypedExpr,
    uses: &mut HashSet<String>,
) {
    match &expr.kind {
        TypedExprKind::Identifier(name) => {
            uses.insert(name.clone());
        }
        TypedExprKind::Binary { left, right, .. } => {
            collect_uses_expr(analysis, left, uses);
            collect_uses_expr(analysis, right, uses);
        }
        TypedExprKind::Unary { operand, .. } => {
            collect_uses_expr(analysis, operand, uses);
        }
        TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
            collect_uses_expr(analysis, left, uses);
            collect_uses_expr(analysis, right, uses);
        }
        TypedExprKind::Call { callee, args } => {
            collect_uses_expr(analysis, callee, uses);
            for arg in args {
                collect_uses_expr(analysis, arg, uses);
            }
        }
        TypedExprKind::Assign { name, value } => {
            uses.insert(name.clone());
            collect_uses_expr(analysis, value, uses);
        }
        TypedExprKind::Grouping(inner) => {
            collect_uses_expr(analysis, inner, uses);
        }
        TypedExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            collect_uses_expr(analysis, condition, uses);
            collect_uses_expr(analysis, then_branch, uses);
            collect_uses_expr(analysis, else_branch, uses);
        }
        TypedExprKind::Lambda(inner) => {
            collect_uses_expr(analysis, inner, uses);
        }
        TypedExprKind::LambdaInner { captures, .. } => {
            for (name, _) in captures {
                uses.insert(name.clone());
                analysis.captured_vars.insert(name.clone());
            }
        }
        TypedExprKind::Member { object, .. } => {
            collect_uses_expr(analysis, object, uses);
        }
        TypedExprKind::ArrayLiteral { elements, .. }
        | TypedExprKind::VecLiteral { elements, .. } => {
            for elem in elements {
                collect_uses_expr(analysis, elem, uses);
            }
        }
        TypedExprKind::ArraySized { size, .. } => {
            collect_uses_expr(analysis, size, uses);
        }
        TypedExprKind::Index { object, index } => {
            collect_uses_expr(analysis, object, uses);
            collect_uses_expr(analysis, index, uses);
        }
        TypedExprKind::IndexAssign {
            object,
            index,
            value,
        } => {
            collect_uses_expr(analysis, object, uses);
            collect_uses_expr(analysis, index, uses);
            collect_uses_expr(analysis, value, uses);
        }
        TypedExprKind::Range { start, end, .. } => {
            if let Some(s) = start {
                collect_uses_expr(analysis, s, uses);
            }
            if let Some(e) = end {
                collect_uses_expr(analysis, e, uses);
            }
        }
        TypedExprKind::Slice { object, range } => {
            collect_uses_expr(analysis, object, uses);
            collect_uses_expr(analysis, range, uses);
        }
        TypedExprKind::FmtString(parts) => {
            for part in parts {
                if let aelys_sema::TypedFmtStringPart::Expr(e) = part {
                    collect_uses_expr(analysis, e, uses);
                }
            }
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, value) in fields {
                collect_uses_expr(analysis, value, uses);
            }
        }
        TypedExprKind::Cast { expr, .. } => {
            collect_uses_expr(analysis, expr, uses);
        }
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Null => {}
    }
}

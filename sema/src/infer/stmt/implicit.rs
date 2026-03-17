use super::TypeInference;
use crate::constraint::{Constraint, ConstraintReason};
use crate::typed_ast::{TypedStmt, TypedStmtKind};
use crate::types::InferType;
use aelys_syntax::{Expr, ExprKind, Stmt, StmtKind, UnaryOp};

impl TypeInference {
    /// Infer statement with implicit return handling
    pub(crate) fn infer_stmt_with_implicit_return(
        &mut self,
        stmt: &Stmt,
        return_type: &InferType,
    ) -> TypedStmt {
        match &stmt.kind {
            aelys_syntax::StmtKind::Expression(expr) => {
                let mut typed_expr = self.infer_expr(expr);
                self.try_narrow_literal(&mut typed_expr, return_type);

                self.constraints.push(Constraint::equal(
                    return_type.clone(),
                    typed_expr.ty.clone(),
                    expr.span,
                    ConstraintReason::Return {
                        func_name: self
                            .env
                            .current_function()
                            .cloned()
                            .unwrap_or_else(|| "<anonymous>".to_string()),
                    },
                ));

                TypedStmt {
                    kind: TypedStmtKind::Return(Some(typed_expr)),
                    span: stmt.span,
                }
            }

            aelys_syntax::StmtKind::If {
                condition,
                then_branch,
                else_branch: Some(else_branch),
            } => {
                let typed_cond = self.infer_expr(condition);

                self.constraints.push(Constraint::equal(
                    typed_cond.ty.clone(),
                    InferType::Bool,
                    condition.span,
                    ConstraintReason::IfCondition,
                ));

                self.env.push_scope();
                let saved_then_literals = self.literal_init_vars.clone();
                let typed_then = self.infer_stmt_with_implicit_return(then_branch, return_type);
                self.env.pop_scope();
                self.literal_init_vars = saved_then_literals;

                self.env.push_scope();
                let saved_else_literals = self.literal_init_vars.clone();
                let typed_else = self.infer_stmt_with_implicit_return(else_branch, return_type);
                self.env.pop_scope();
                self.literal_init_vars = saved_else_literals;

                TypedStmt {
                    kind: TypedStmtKind::If {
                        condition: typed_cond,
                        then_branch: Box::new(typed_then),
                        else_branch: Some(Box::new(typed_else)),
                    },
                    span: stmt.span,
                }
            }

            aelys_syntax::StmtKind::Block(stmts) if !stmts.is_empty() => {
                self.env.push_scope();
                let prefix = self
                    .env
                    .current_function()
                    .map_or_else(String::new, Clone::clone);
                self.collect_signatures(stmts, &prefix);
                let saved_block_literals = self.literal_init_vars.clone();

                let mut typed_stmts: Vec<TypedStmt> = stmts[..stmts.len() - 1]
                    .iter()
                    .map(|s| self.infer_stmt(s))
                    .collect();

                let typed_last =
                    self.infer_stmt_with_implicit_return(&stmts[stmts.len() - 1], return_type);
                typed_stmts.push(typed_last);

                self.env.pop_scope();
                self.literal_init_vars = saved_block_literals;

                TypedStmt {
                    kind: TypedStmtKind::Block(typed_stmts),
                    span: stmt.span,
                }
            }

            _ => {
                let typed_stmt = self.infer_stmt(stmt);
                if !stmt_guarantees_return(stmt) {
                    self.constraints.push(Constraint::equal(
                        return_type.clone(),
                        InferType::Null,
                        stmt.span,
                        ConstraintReason::Return {
                            func_name: self
                                .env
                                .current_function()
                                .cloned()
                                .unwrap_or_else(|| "<anonymous>".to_string()),
                        },
                    ));
                }
                typed_stmt
            }
        }
    }
}

fn stmt_guarantees_return(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) => true,
        StmtKind::Block(stmts) => stmts.iter().any(stmt_guarantees_return),
        StmtKind::If {
            then_branch,
            else_branch: Some(else_branch),
            ..
        } => stmt_guarantees_return(then_branch) && stmt_guarantees_return(else_branch),
        StmtKind::For {
            start,
            end,
            inclusive,
            step,
            body,
            ..
        } => {
            for_loop_executes_at_least_once(start, end, *inclusive, step.as_ref().as_ref())
                && stmt_guarantees_return(body)
        }
        // `while true { ... return ... }` is an infinite loop that can only
        // exit via `return` (or loop forever).  If the condition is `true`
        // and the body contains at least one `return` and no `break`, the
        // loop never falls through to the next statement.
        StmtKind::While { condition, body } => {
            is_const_true(condition)
                && stmt_contains_return(body)
                && !stmt_contains_break(body)
        }
        _ => false,
    }
}

fn is_const_true(expr: &Expr) -> bool {
    match &expr.kind {
        ExprKind::Bool(true) => true,
        ExprKind::Grouping(inner) => is_const_true(inner),
        _ => false,
    }
}

/// Check if a statement contains a `return` anywhere (recursively).
fn stmt_contains_return(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Return(_) => true,
        StmtKind::Block(stmts) => stmts.iter().any(stmt_contains_return),
        StmtKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            stmt_contains_return(then_branch)
                || else_branch.as_ref().is_some_and(|e| stmt_contains_return(e))
        }
        StmtKind::While { body, .. }
        | StmtKind::For { body, .. }
        | StmtKind::ForEach { body, .. } => stmt_contains_return(body),
        _ => false,
    }
}

/// Check if a statement contains a `break` at the current loop level
/// (does NOT recurse into nested loops, since break only affects the
/// innermost loop).
fn stmt_contains_break(stmt: &Stmt) -> bool {
    match &stmt.kind {
        StmtKind::Break => true,
        StmtKind::Block(stmts) => stmts.iter().any(stmt_contains_break),
        StmtKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            stmt_contains_break(then_branch)
                || else_branch.as_ref().is_some_and(|e| stmt_contains_break(e))
        }
        // Don't recurse into nested loops — break in a nested loop
        // doesn't affect the outer while-true.
        StmtKind::While { .. } | StmtKind::For { .. } | StmtKind::ForEach { .. } => false,
        _ => false,
    }
}

fn for_loop_executes_at_least_once(
    start: &Expr,
    end: &Expr,
    inclusive: bool,
    step: Option<&Expr>,
) -> bool {
    let Some(start_value) = int_literal_value(start) else {
        return false;
    };
    let Some(end_value) = int_literal_value(end) else {
        return false;
    };

    if let Some(step_expr) = step {
        let Some(step_value) = int_literal_value(step_expr) else {
            return false;
        };
        if step_value == 0 {
            return false;
        }
        return if step_value > 0 {
            if inclusive {
                start_value <= end_value
            } else {
                start_value < end_value
            }
        } else if inclusive {
            start_value >= end_value
        } else {
            start_value > end_value
        };
    }

    if inclusive {
        start_value <= end_value
    } else {
        start_value < end_value
    }
}

fn int_literal_value(expr: &Expr) -> Option<i64> {
    match &expr.kind {
        ExprKind::Int(value) => Some(*value),
        ExprKind::Grouping(inner) => int_literal_value(inner),
        ExprKind::Unary {
            op: UnaryOp::Neg,
            operand,
        } => int_literal_value(operand).and_then(|value| value.checked_neg()),
        _ => None,
    }
}

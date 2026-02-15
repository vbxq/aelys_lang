use super::DeadCodeEliminator;
use aelys_sema::{TypedExpr, TypedExprKind, TypedFunction, TypedStmt, TypedStmtKind};

impl DeadCodeEliminator {
    // terminators: return, break, continue (nothing after them runs)
    fn is_terminator(stmt: &TypedStmt) -> bool {
        match &stmt.kind {
            TypedStmtKind::Return(_) | TypedStmtKind::Break | TypedStmtKind::Continue => true,
            TypedStmtKind::Block(stmts) => stmts.last().map(Self::is_terminator).unwrap_or(false),
            TypedStmtKind::If {
                then_branch,
                else_branch,
                ..
            } => {
                Self::is_terminator(then_branch)
                    && else_branch
                        .as_ref()
                        .map(|e| Self::is_terminator(e))
                        .unwrap_or(false)
            }
            _ => false,
        }
    }

    pub(super) fn is_const_bool(expr: &TypedExpr) -> Option<bool> {
        match &expr.kind {
            TypedExprKind::Bool(b) => Some(*b),
            TypedExprKind::Grouping(inner) => Self::is_const_bool(inner),
            _ => None,
        }
    }

    pub(super) fn eliminate_in_block(&mut self, stmts: &mut Vec<TypedStmt>) -> bool {
        for stmt in stmts.iter_mut() {
            self.eliminate_in_stmt(stmt);
        }

        // cut off everything after a terminator
        let mut found_terminator = false;
        stmts.retain(|stmt| {
            if found_terminator {
                self.stats.dead_code_eliminated += 1;
                return false;
            }
            if Self::is_terminator(stmt) {
                found_terminator = true;
            }
            true
        });

        // also remove empty blocks
        stmts.retain(|stmt| {
            if let TypedStmtKind::Block(inner) = &stmt.kind
                && inner.is_empty()
            {
                self.stats.dead_code_eliminated += 1;
                return false;
            }
            true
        });

        stmts.is_empty()
    }

    pub(super) fn eliminate_in_stmt(&mut self, stmt: &mut TypedStmt) {
        match &mut stmt.kind {
            TypedStmtKind::Block(stmts) => {
                self.eliminate_in_block(stmts);
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                // if condition is constant, replace whole if with the taken branch
                if let Some(cond_value) = Self::is_const_bool(condition) {
                    let branch = if cond_value {
                        self.eliminate_in_stmt(then_branch);
                        Some(then_branch)
                    } else if let Some(else_b) = else_branch {
                        self.eliminate_in_stmt(else_b);
                        Some(else_b)
                    } else {
                        None
                    };

                    if let Some(branch) = branch {
                        // unwrap single-statement blocks to preserve if-else return semantics
                        // (if-else returns values, raw blocks don't in Aelys)
                        if let TypedStmtKind::Block(inner) = &branch.kind {
                            if inner.len() == 1 {
                                *stmt = inner[0].clone();
                            } else {
                                *stmt = (**branch).clone();
                            }
                        } else {
                            *stmt = (**branch).clone();
                        }
                    } else {
                        stmt.kind = TypedStmtKind::Block(vec![]);
                    }
                    self.stats.branches_eliminated += 1;
                    return;
                }
                self.eliminate_in_stmt(then_branch);
                if let Some(else_b) = else_branch {
                    self.eliminate_in_stmt(else_b);
                }
            }
            TypedStmtKind::While { condition, body } => {
                // while false { } -> nothing
                if Self::is_const_bool(condition) == Some(false) {
                    stmt.kind = TypedStmtKind::Block(vec![]);
                    self.stats.branches_eliminated += 1;
                    return;
                }
                self.eliminate_in_stmt(body);
            }
            TypedStmtKind::For { body, .. } => self.eliminate_in_stmt(body),
            TypedStmtKind::ForEach { body, .. } => self.eliminate_in_stmt(body),
            TypedStmtKind::Function(func) => self.eliminate_in_function(func),
            TypedStmtKind::Expression(expr) => self.eliminate_in_expr(expr),
            TypedStmtKind::Let { initializer, .. } => self.eliminate_in_expr(initializer),
            TypedStmtKind::Return(_)
            | TypedStmtKind::Break
            | TypedStmtKind::Continue
            | TypedStmtKind::Needs(_) => {}
        }
    }

    fn eliminate_in_function(&mut self, func: &mut TypedFunction) {
        self.eliminate_in_block(&mut func.body);
    }
}

use super::ConstantFolder;
use aelys_sema::{TypedFunction, TypedStmt, TypedStmtKind};

impl ConstantFolder {
    pub(super) fn optimize_stmt(&mut self, stmt: &mut TypedStmt) {
        match &mut stmt.kind {
            TypedStmtKind::Expression(expr) => self.optimize_expr(expr),
            TypedStmtKind::Let { initializer, .. } => self.optimize_expr(initializer),
            TypedStmtKind::Block(stmts) => {
                for s in stmts { self.optimize_stmt(s); }
            }
            TypedStmtKind::If { condition, then_branch, else_branch } => {
                self.optimize_expr(condition);
                self.optimize_stmt(then_branch);
                if let Some(else_b) = else_branch { self.optimize_stmt(else_b); }
            }
            TypedStmtKind::While { condition, body } => {
                self.optimize_expr(condition);
                self.optimize_stmt(body);
            }
            TypedStmtKind::For { start, end, step, body, .. } => {
                self.optimize_expr(start);
                self.optimize_expr(end);
                if let Some(s) = step { self.optimize_expr(s); }
                self.optimize_stmt(body);
            }
            TypedStmtKind::Return(Some(expr)) => self.optimize_expr(expr),
            TypedStmtKind::Function(func) => self.optimize_function(func),
            TypedStmtKind::Return(None) | TypedStmtKind::Break
            | TypedStmtKind::Continue | TypedStmtKind::Needs(_) => {}
        }
    }

    fn optimize_function(&mut self, func: &mut TypedFunction) {
        for stmt in &mut func.body { self.optimize_stmt(stmt); }
    }
}

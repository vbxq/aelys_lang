use super::GlobalConstantPropagator;
use aelys_sema::{TypedExpr, TypedExprKind, TypedStmt, TypedStmtKind};

impl GlobalConstantPropagator {
    // can this expr be evaluated at compile time?
    pub(super) fn is_constant_expr(&self, expr: &TypedExpr) -> bool {
        match &expr.kind {
            TypedExprKind::Int(_) | TypedExprKind::Float(_) | TypedExprKind::Bool(_)
            | TypedExprKind::String(_) | TypedExprKind::Null => true,
            TypedExprKind::Identifier(name) => self.constants.contains_key(name),
            TypedExprKind::Binary { left, right, .. } => {
                self.is_constant_expr(left) && self.is_constant_expr(right)
            }
            TypedExprKind::Unary { operand, .. } => self.is_constant_expr(operand),
            TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
                self.is_constant_expr(left) && self.is_constant_expr(right)
            }
            TypedExprKind::Grouping(inner) => self.is_constant_expr(inner),
            TypedExprKind::If { condition, then_branch, else_branch } => {
                self.is_constant_expr(condition) &&
                self.is_constant_expr(then_branch) &&
                self.is_constant_expr(else_branch)
            }
            // calls, assigns, lambdas, member access - not constant
            _ => false,
        }
    }

    pub(super) fn collect_global_constants(&mut self, stmts: &[TypedStmt]) {
        // iterate until fixpoint (handles `let B = A + 1` after `let A = 1`)
        // cap at 10 to avoid infinite loops on weird edge cases
        for _ in 0..10 {
            let prev_count = self.constants.len();

            for stmt in stmts {
                if let TypedStmtKind::Let { name, mutable, initializer, .. } = &stmt.kind {
                    if *mutable || self.constants.contains_key(name) { continue; }
                    if self.is_constant_expr(initializer) {
                        let mut resolved = initializer.clone();
                        self.substitute_in_expr_for_collection(&mut resolved);
                        self.constants.insert(name.clone(), resolved);
                    }
                }
            }

            if self.constants.len() == prev_count { break; }
        }
    }

    // substitute known constants during collection phase (for chained constants)
    pub(super) fn substitute_in_expr_for_collection(&self, expr: &mut TypedExpr) {
        match &mut expr.kind {
            TypedExprKind::Identifier(name) => {
                if let Some(c) = self.constants.get(name) {
                    *expr = TypedExpr::new(c.kind.clone(), c.ty.clone(), expr.span);
                }
            }
            TypedExprKind::Binary { left, right, .. } => {
                self.substitute_in_expr_for_collection(left);
                self.substitute_in_expr_for_collection(right);
            }
            TypedExprKind::Unary { operand, .. } => self.substitute_in_expr_for_collection(operand),
            TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
                self.substitute_in_expr_for_collection(left);
                self.substitute_in_expr_for_collection(right);
            }
            TypedExprKind::Grouping(inner) => self.substitute_in_expr_for_collection(inner),
            TypedExprKind::If { condition, then_branch, else_branch } => {
                self.substitute_in_expr_for_collection(condition);
                self.substitute_in_expr_for_collection(then_branch);
                self.substitute_in_expr_for_collection(else_branch);
            }
            _ => {}
        }
    }
}

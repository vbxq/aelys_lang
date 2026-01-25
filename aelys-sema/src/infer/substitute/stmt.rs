use super::super::TypeInference;
use crate::typed_ast::{TypedStmt, TypedStmtKind};
use crate::unify::Substitution;

impl TypeInference {
    /// Apply substitution to a statement
    pub(super) fn apply_substitution_stmt(
        &self,
        stmt: &TypedStmt,
        subst: &Substitution,
    ) -> TypedStmt {
        let kind = match &stmt.kind {
            TypedStmtKind::Expression(expr) => {
                TypedStmtKind::Expression(self.apply_substitution_expr(expr, subst))
            }
            TypedStmtKind::Let {
                name,
                mutable,
                initializer,
                var_type,
                is_pub,
            } => TypedStmtKind::Let {
                name: name.clone(),
                mutable: *mutable,
                initializer: self.apply_substitution_expr(initializer, subst),
                var_type: subst.apply(var_type),
                is_pub: *is_pub,
            },
            TypedStmtKind::Block(stmts) => {
                TypedStmtKind::Block(self.apply_substitution_stmts(stmts, subst))
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => TypedStmtKind::If {
                condition: self.apply_substitution_expr(condition, subst),
                then_branch: Box::new(self.apply_substitution_stmt(then_branch, subst)),
                else_branch: else_branch
                    .as_ref()
                    .map(|e| Box::new(self.apply_substitution_stmt(e, subst))),
            },
            TypedStmtKind::While { condition, body } => TypedStmtKind::While {
                condition: self.apply_substitution_expr(condition, subst),
                body: Box::new(self.apply_substitution_stmt(body, subst)),
            },
            TypedStmtKind::For {
                iterator,
                start,
                end,
                inclusive,
                step,
                body,
            } => TypedStmtKind::For {
                iterator: iterator.clone(),
                start: self.apply_substitution_expr(start, subst),
                end: self.apply_substitution_expr(end, subst),
                inclusive: *inclusive,
                step: step
                    .as_ref()
                    .map(|s| self.apply_substitution_expr(s, subst)),
                body: Box::new(self.apply_substitution_stmt(body, subst)),
            },
            TypedStmtKind::Return(expr) => TypedStmtKind::Return(
                expr.as_ref()
                    .map(|e| self.apply_substitution_expr(e, subst)),
            ),
            TypedStmtKind::Break => TypedStmtKind::Break,
            TypedStmtKind::Continue => TypedStmtKind::Continue,
            TypedStmtKind::Function(func) => {
                TypedStmtKind::Function(self.apply_substitution_func(func, subst))
            }
            TypedStmtKind::Needs(needs) => TypedStmtKind::Needs(needs.clone()),
        };

        TypedStmt {
            kind,
            span: stmt.span,
        }
    }
}

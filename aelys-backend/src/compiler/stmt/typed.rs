use super::Compiler;
use aelys_common::Result;

impl Compiler {
    pub fn compile_typed_stmt(&mut self, stmt: &aelys_sema::TypedStmt) -> Result<()> {
        use aelys_sema::TypedStmtKind;

        match &stmt.kind {
            TypedStmtKind::Expression(expr) => {
                let reg = self.alloc_register()?;
                self.compile_typed_expr(expr, reg)?;
                self.free_register(reg);
                Ok(())
            }
            TypedStmtKind::Let {
                name,
                mutable,
                initializer,
                var_type,
                is_pub,
            } => {
                let resolved_type = aelys_sema::ResolvedType::from_infer_type(var_type);
                self.compile_typed_let(
                    name,
                    *mutable,
                    initializer,
                    &resolved_type,
                    *is_pub,
                    stmt.span,
                )
            }
            TypedStmtKind::Block(stmts) => {
                self.begin_scope();
                for s in stmts {
                    self.compile_typed_stmt(s)?;
                }
                self.end_scope();
                Ok(())
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.compile_typed_if_stmt(
                condition,
                then_branch,
                else_branch.as_deref(),
                stmt.span,
            ),
            TypedStmtKind::While { condition, body } => {
                self.compile_typed_while(condition, body, stmt.span)
            }
            TypedStmtKind::For {
                iterator,
                start,
                end,
                inclusive,
                step,
                body,
            } => self.compile_typed_for(
                iterator,
                start,
                end,
                *inclusive,
                step.as_ref().as_ref(),
                body,
                stmt.span,
            ),
            TypedStmtKind::Return(expr) => self.compile_typed_return(expr.as_ref(), stmt.span),
            TypedStmtKind::Break => self.compile_break(stmt.span),
            TypedStmtKind::Continue => self.compile_continue(stmt.span),
            TypedStmtKind::Function(func) => self.compile_typed_function(func),
            TypedStmtKind::Needs(_needs) => Ok(()),
        }
    }
}

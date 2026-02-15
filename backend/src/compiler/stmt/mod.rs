mod control_flow;
mod control_if;
mod decl;
mod expression;
mod looping;
mod typed;

use super::Compiler;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::{Stmt, StmtKind};

impl Compiler {
    pub fn compile_stmt(&mut self, stmt: &Stmt) -> Result<()> {
        match &stmt.kind {
            StmtKind::Expression(expr) => self.compile_expression_stmt(expr),
            StmtKind::Let {
                name,
                mutable,
                type_annotation,
                initializer,
                is_pub,
            } => self.compile_let(
                name,
                *mutable,
                type_annotation.as_ref(),
                initializer,
                *is_pub,
            ),
            StmtKind::Block(stmts) => self.compile_block(stmts),
            StmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.compile_if(condition, then_branch, else_branch.as_deref()),
            StmtKind::While { condition, body } => self.compile_while(condition, body),
            StmtKind::For {
                iterator,
                start,
                end,
                inclusive,
                step,
                body,
            } => self.compile_for(
                iterator,
                start,
                end,
                *inclusive,
                step.as_ref().as_ref(),
                body,
                stmt.span,
            ),
            StmtKind::ForEach { .. } => {
                // ForEach is handled through typed compilation path only
                Ok(())
            }
            StmtKind::Break => self.compile_break(stmt.span),
            StmtKind::Continue => self.compile_continue(stmt.span),
            StmtKind::Return(expr) => self.compile_return(expr.as_ref(), stmt.span),
            StmtKind::Function(func) => self.compile_function(func),
            StmtKind::Needs(needs) => self.compile_needs(needs, stmt.span),
        }
    }

    pub fn compile_block(&mut self, stmts: &[Stmt]) -> Result<()> {
        self.begin_scope();
        for s in stmts {
            self.compile_stmt(s)?;
        }
        self.end_scope();
        Ok(())
    }

    // imports are resolved earlier, nothing to emit here
    pub fn compile_needs(&mut self, _: &aelys_syntax::ast::NeedsStmt, _: Span) -> Result<()> {
        Ok(())
    }
}

use super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::{Stmt, StmtKind};

impl Compiler {
    pub fn compile_if_branch_for_return(&mut self, branch: &Stmt, dest: u8) -> Result<()> {
        match &branch.kind {
            StmtKind::Block(stmts) => {
                self.begin_scope();
                if stmts.is_empty() {
                    self.emit_a(OpCode::LoadNull, dest, 0, 0, branch.span);
                } else {
                    for stmt in &stmts[..stmts.len() - 1] {
                        self.compile_stmt(stmt)?;
                    }
                    let last = &stmts[stmts.len() - 1];
                    match &last.kind {
                        StmtKind::Expression(expr) => {
                            self.compile_expr(expr, dest)?;
                        }
                        StmtKind::If {
                            condition,
                            then_branch,
                            else_branch,
                        } if else_branch.is_some() => {
                            let cond_reg = self.alloc_register()?;
                            self.compile_expr(condition, cond_reg)?;
                            let jump_to_else =
                                self.emit_jump_if(OpCode::JumpIfNot, cond_reg, condition.span);
                            self.free_register(cond_reg);

                            self.compile_if_branch_for_return(then_branch, dest)?;
                            let jump_to_end = self.emit_jump(OpCode::Jump, then_branch.span);
                            self.patch_jump(jump_to_else);

                            if let Some(else_branch) = else_branch.as_ref() {
                                self.compile_if_branch_for_return(else_branch, dest)?;
                            }
                            self.patch_jump(jump_to_end);
                        }
                        _ => {
                            self.compile_stmt(last)?;
                            self.emit_a(OpCode::LoadNull, dest, 0, 0, branch.span);
                        }
                    }
                }
                self.end_scope();
            }
            StmtKind::Expression(expr) => {
                self.compile_expr(expr, dest)?;
            }
            _ => {
                self.compile_stmt(branch)?;
                self.emit_a(OpCode::LoadNull, dest, 0, 0, branch.span);
            }
        }
        Ok(())
    }

    pub fn compile_typed_if_branch_for_return(
        &mut self,
        branch: &aelys_sema::TypedStmt,
        dest: u8,
    ) -> Result<()> {
        use aelys_sema::TypedStmtKind;

        match &branch.kind {
            TypedStmtKind::Block(stmts) => {
                self.begin_scope();
                if stmts.is_empty() {
                    self.emit_a(OpCode::LoadNull, dest, 0, 0, branch.span);
                } else {
                    for stmt in &stmts[..stmts.len() - 1] {
                        self.compile_typed_stmt(stmt)?;
                    }
                    let last = &stmts[stmts.len() - 1];
                    match &last.kind {
                        TypedStmtKind::Expression(expr) => {
                            self.compile_typed_expr(expr, dest)?;
                        }
                        TypedStmtKind::If {
                            condition,
                            then_branch,
                            else_branch,
                        } if else_branch.is_some() => {
                            let cond_reg = self.alloc_register()?;
                            self.compile_typed_expr(condition, cond_reg)?;
                            let jump_to_else =
                                self.emit_jump_if(OpCode::JumpIfNot, cond_reg, condition.span);
                            self.free_register(cond_reg);

                            self.compile_typed_if_branch_for_return(then_branch, dest)?;
                            let jump_to_end = self.emit_jump(OpCode::Jump, then_branch.span);
                            self.patch_jump(jump_to_else);

                            if let Some(else_branch) = else_branch.as_ref() {
                                self.compile_typed_if_branch_for_return(else_branch, dest)?;
                            }
                            self.patch_jump(jump_to_end);
                        }
                        _ => {
                            self.compile_typed_stmt(last)?;
                            self.emit_a(OpCode::LoadNull, dest, 0, 0, branch.span);
                        }
                    }
                }
                self.end_scope();
            }
            TypedStmtKind::Expression(expr) => {
                self.compile_typed_expr(expr, dest)?;
            }
            _ => {
                self.compile_typed_stmt(branch)?;
                self.emit_a(OpCode::LoadNull, dest, 0, 0, branch.span);
            }
        }
        Ok(())
    }

    pub fn compile_if(
        &mut self,
        condition: &aelys_syntax::ast::Expr,
        then_branch: &Stmt,
        else_branch: Option<&Stmt>,
    ) -> Result<()> {
        let cond_reg = self.alloc_register()?;
        self.compile_expr(condition, cond_reg)?;
        let jump_to_else = self.emit_jump_if(OpCode::JumpIfNot, cond_reg, condition.span);

        self.free_register(cond_reg);
        self.compile_stmt(then_branch)?;

        if let Some(else_branch) = else_branch {
            let jump_to_end = self.emit_jump(OpCode::Jump, then_branch.span);
            self.patch_jump(jump_to_else);
            self.compile_stmt(else_branch)?;
            self.patch_jump(jump_to_end);
        } else {
            self.patch_jump(jump_to_else);
        }

        Ok(())
    }

    pub fn compile_typed_if_stmt(
        &mut self,
        condition: &aelys_sema::TypedExpr,
        then_branch: &aelys_sema::TypedStmt,
        else_branch: Option<&aelys_sema::TypedStmt>,
        span: Span,
    ) -> Result<()> {
        let cond_reg = self.alloc_register()?;
        self.compile_typed_expr(condition, cond_reg)?;

        let else_jump = self.emit_jump_if(OpCode::JumpIfNot, cond_reg, span);
        self.free_register(cond_reg);

        self.compile_typed_stmt(then_branch)?;

        if let Some(else_stmt) = else_branch {
            let end_jump = self.emit_jump(OpCode::Jump, span);
            self.patch_jump(else_jump);
            self.compile_typed_stmt(else_stmt)?;
            self.patch_jump(end_jump);
        } else {
            self.patch_jump(else_jump);
        }

        Ok(())
    }
}

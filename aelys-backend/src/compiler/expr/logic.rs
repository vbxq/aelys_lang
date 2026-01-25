use super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::Expr;

impl Compiler {
    // Short-circuit: if left is false, skip right
    pub fn compile_and(&mut self, left: &Expr, right: &Expr, dest: u8, span: Span) -> Result<()> {
        self.compile_expr(left, dest)?;
        let jump = self.emit_jump_if(OpCode::JumpIfNot, dest, span);
        self.compile_expr(right, dest)?;
        self.patch_jump(jump);
        Ok(())
    }

    // Short-circuit: if left is true, skip right
    pub fn compile_or(&mut self, left: &Expr, right: &Expr, dest: u8, span: Span) -> Result<()> {
        self.compile_expr(left, dest)?;
        let jump = self.emit_jump_if(OpCode::JumpIf, dest, span);
        self.compile_expr(right, dest)?;
        self.patch_jump(jump);
        Ok(())
    }
}

use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;

impl Compiler {
    pub(super) fn compile_typed_and(
        &mut self,
        left: &aelys_sema::TypedExpr,
        right: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        self.compile_typed_expr(left, dest)?;
        let jump_false = self.emit_jump_if(OpCode::JumpIfNot, dest, span);
        self.compile_typed_expr(right, dest)?;
        self.patch_jump(jump_false);
        Ok(())
    }

    pub(super) fn compile_typed_or(
        &mut self,
        left: &aelys_sema::TypedExpr,
        right: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        self.compile_typed_expr(left, dest)?;
        let jump_true = self.emit_jump_if(OpCode::JumpIf, dest, span);
        self.compile_typed_expr(right, dest)?;
        self.patch_jump(jump_true);
        Ok(())
    }
}

use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;

impl Compiler {
    pub(super) fn compile_typed_if_expr(
        &mut self,
        condition: &aelys_sema::TypedExpr,
        then_branch: &aelys_sema::TypedExpr,
        else_branch: &aelys_sema::TypedExpr,
        dest: u8,
    ) -> Result<()> {
        let cond_reg = self.alloc_register()?;
        self.compile_typed_expr(condition, cond_reg)?;

        let else_jump = self.emit_jump_if(OpCode::JumpIfNot, cond_reg, condition.span);
        self.free_register(cond_reg);

        self.compile_typed_expr(then_branch, dest)?;
        let end_jump = self.emit_jump(OpCode::Jump, then_branch.span);

        self.patch_jump(else_jump);
        self.compile_typed_expr(else_branch, dest)?;
        self.patch_jump(end_jump);

        Ok(())
    }
}

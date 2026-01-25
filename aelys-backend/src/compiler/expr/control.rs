use super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::ast::Expr;

impl Compiler {
    // ternary: cond ? then : else
    pub fn compile_if_expr(
        &mut self,
        cond: &Expr,
        then_: &Expr,
        else_: &Expr,
        dest: u8,
    ) -> Result<()> {
        let tmp = self.alloc_register()?;
        self.compile_expr(cond, tmp)?;
        let jmp_else = self.emit_jump_if(OpCode::JumpIfNot, tmp, cond.span);
        self.free_register(tmp);

        self.compile_expr(then_, dest)?;
        let jmp_end = self.emit_jump(OpCode::Jump, then_.span);
        self.patch_jump(jmp_else);

        self.compile_expr(else_, dest)?;
        self.patch_jump(jmp_end);
        Ok(())
    }
}

use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::Expr;

impl Compiler {
    pub fn compile_call_generic(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let nargs = args.len();
        let func_reg = self.alloc_consecutive_registers_for_call(nargs as u8 + 1, span)?;

        for i in 0..=nargs {
            let reg = func_reg + i as u8;
            self.register_pool[reg as usize] = true;
            if reg >= self.next_register {
                self.next_register = reg + 1;
            }
        }

        self.compile_expr(callee, func_reg)?;

        for (i, arg) in args.iter().enumerate() {
            let arg_reg = func_reg + 1 + i as u8;
            self.compile_expr(arg, arg_reg)?;
        }

        self.emit_c(OpCode::Call, dest, func_reg, args.len() as u8, span);

        for i in (0..=nargs).rev() {
            let reg = func_reg + i as u8;
            self.register_pool[reg as usize] = false;
        }

        Ok(())
    }
}

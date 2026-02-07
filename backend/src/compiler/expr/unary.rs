use super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::{Expr, UnaryOp};

impl Compiler {
    // could probably optimize -literal to just negate at compile time...
    pub fn compile_unary(
        &mut self,
        op: UnaryOp,
        operand: &Expr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let operand_reg = self.alloc_register()?;

        self.compile_expr(operand, operand_reg)?;

        let opcode = match op {
            UnaryOp::Neg => OpCode::Neg,
            UnaryOp::Not => OpCode::Not,
            UnaryOp::BitNot => OpCode::BitNot,
        };

        self.emit_a(opcode, dest, operand_reg, 0, span);
        self.free_register(operand_reg);

        Ok(())
    }
}

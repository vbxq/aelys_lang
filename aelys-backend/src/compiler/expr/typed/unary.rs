use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::UnaryOp;

impl Compiler {
    pub(super) fn compile_typed_unary(
        &mut self,
        op: UnaryOp,
        operand: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        self.compile_typed_expr(operand, dest)?;

        let opcode = match op {
            UnaryOp::Neg => OpCode::Neg,
            UnaryOp::Not => OpCode::Not,
            UnaryOp::BitNot => OpCode::BitNot,
        };
        self.emit_a(opcode, dest, dest, 0, span);

        Ok(())
    }
}

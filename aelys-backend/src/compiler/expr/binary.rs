use super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::{BinaryOp, Expr, ExprKind};

impl Compiler {
    // Binary ops have a bunch of special cases for immediate values.
    // `x + 5` becomes AddI instead of LoadK + Add, saves a register and an instruction.
    pub fn compile_binary(
        &mut self,
        left: &Expr,
        op: BinaryOp,
        right: &Expr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        // Right operand is small constant: use immediate instructions
        if let ExprKind::Int(n) = &right.kind
            && *n >= 0
            && *n <= 255
            && let Some(left_reg) = self.get_local_register(left)
        {
            match op {
                BinaryOp::Add => {
                    self.emit_a(OpCode::AddI, dest, left_reg, *n as u8, span);
                    return Ok(());
                }
                BinaryOp::Sub => {
                    self.emit_a(OpCode::SubI, dest, left_reg, *n as u8, span);
                    return Ok(());
                }
                BinaryOp::Shl => {
                    self.emit_a(OpCode::ShlIImm, dest, left_reg, *n as u8, span);
                    return Ok(());
                }
                BinaryOp::Shr => {
                    self.emit_a(OpCode::ShrIImm, dest, left_reg, *n as u8, span);
                    return Ok(());
                }
                BinaryOp::BitAnd => {
                    self.emit_a(OpCode::AndIImm, dest, left_reg, *n as u8, span);
                    return Ok(());
                }
                BinaryOp::BitOr => {
                    self.emit_a(OpCode::OrIImm, dest, left_reg, *n as u8, span);
                    return Ok(());
                }
                BinaryOp::BitXor => {
                    self.emit_a(OpCode::XorIImm, dest, left_reg, *n as u8, span);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Commutative ops: `5 + x` can also use AddI (swap operands)
        if let ExprKind::Int(n) = &left.kind
            && *n >= 0
            && *n <= 255
            && op == BinaryOp::Add
            && let Some(right_reg) = self.get_local_register(right)
        {
            self.emit_a(OpCode::AddI, dest, right_reg, *n as u8, span);
            return Ok(());
        }

        // Same for bitwise ops - they're all commutative
        if let ExprKind::Int(n) = &left.kind
            && *n >= 0
            && *n <= 255
            && let Some(right_reg) = self.get_local_register(right)
        {
            match op {
                BinaryOp::BitAnd => {
                    self.emit_a(OpCode::AndIImm, dest, right_reg, *n as u8, span);
                    return Ok(());
                }
                BinaryOp::BitOr => {
                    self.emit_a(OpCode::OrIImm, dest, right_reg, *n as u8, span);
                    return Ok(());
                }
                BinaryOp::BitXor => {
                    self.emit_a(OpCode::XorIImm, dest, right_reg, *n as u8, span);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Generic path: evaluate both sides into registers.
        // Reuse existing registers when possible to avoid spilling.
        let left_local = self.get_local_register(left);
        let right_local = self.get_local_register(right);

        let (left_reg, left_allocated) = match left_local {
            Some(r) => (r, false),
            None => (self.alloc_register()?, true),
        };

        let (right_reg, right_allocated) = match right_local {
            Some(r) => (r, false),
            None => (self.alloc_register()?, true),
        };

        if left_allocated {
            self.compile_expr(left, left_reg)?;
        }
        if right_allocated {
            self.compile_expr(right, right_reg)?;
        }

        let opcode = match op {
            BinaryOp::Add => OpCode::Add,
            BinaryOp::Sub => OpCode::Sub,
            BinaryOp::Mul => OpCode::Mul,
            BinaryOp::Div => OpCode::Div,
            BinaryOp::Mod => OpCode::Mod,
            BinaryOp::Lt => OpCode::Lt,
            BinaryOp::Le => OpCode::Le,
            BinaryOp::Gt => OpCode::Gt,
            BinaryOp::Ge => OpCode::Ge,
            BinaryOp::Eq => OpCode::Eq,
            BinaryOp::Ne => OpCode::Ne,
            BinaryOp::Shl => OpCode::Shl,
            BinaryOp::Shr => OpCode::Shr,
            BinaryOp::BitAnd => OpCode::BitAnd,
            BinaryOp::BitOr => OpCode::BitOr,
            BinaryOp::BitXor => OpCode::BitXor,
        };
        self.emit_a(opcode, dest, left_reg, right_reg, span);

        if right_allocated {
            self.free_register(right_reg);
        }
        if left_allocated {
            self.free_register(left_reg);
        }

        Ok(())
    }
}

use super::super::{Compiler, LoopContext};
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::ast::{BinaryOp, Expr, ExprKind, Stmt};

impl Compiler {
    pub fn compile_while(&mut self, condition: &Expr, body: &Stmt) -> Result<()> {
        // Fast path: `while i < bound` is super common, use specialized opcode
        if let Some(optimized) = self.try_compile_while_loop_lt(condition, body)? {
            return Ok(optimized);
        }
        self.compile_while_generic(condition, body)
    }

    // WhileLoopLt: fuses comparison + conditional jump into one instruction.
    // Only works for `while local_var < expr` pattern - anything else falls back to generic.
    pub fn try_compile_while_loop_lt(
        &mut self,
        condition: &Expr,
        body: &Stmt,
    ) -> Result<Option<()>> {
        if let ExprKind::Binary {
            op: BinaryOp::Lt,
            left,
            right,
        } = &condition.kind
        {
            if let ExprKind::Identifier(name) = &left.kind {
                if let Some((iter_reg, _)) = self.resolve_variable(name) {
                    // Need the bound in the register right after the iterator.
                    // WhileLoopLt expects them adjacent. If that register is taken, bail.
                    // FIXME: could spill/move but probably not worth the complexity
                    let bound_reg = match iter_reg.checked_add(1) {
                        Some(r) if (r as usize) < self.register_pool.len() => r,
                        _ => return Ok(None),
                    };

                    if self.register_pool[bound_reg as usize] {
                        return Ok(None);
                    }

                    self.register_pool[bound_reg as usize] = true;
                    if bound_reg >= self.next_register {
                        self.next_register = bound_reg + 1;
                    }

                    self.compile_expr(right, bound_reg)?;

                    let loop_start = self.current_offset();

                    self.loop_stack.push(LoopContext {
                        start: loop_start,
                        break_jumps: Vec::new(),
                        continue_jumps: Vec::new(),
                        is_for_loop: false,
                    });

                    self.emit_b(OpCode::WhileLoopLt, iter_reg, 1, condition.span);
                    let jump_to_end = self.emit_jump(OpCode::Jump, condition.span);

                    self.compile_stmt(body)?;

                    let jump_dist = (self.current_offset() - loop_start + 1) as i16;
                    self.emit_b(OpCode::Jump, 0, -jump_dist, body.span);

                    self.patch_jump(jump_to_end);

                    if let Some(loop_ctx) = self.loop_stack.pop() {
                        for break_jump in loop_ctx.break_jumps {
                            self.patch_jump(break_jump);
                        }
                    }

                    self.register_pool[bound_reg as usize] = false;

                    return Ok(Some(()));
                }
            }
        }

        Ok(None)
    }

    // Fallback for arbitrary conditions. Less efficient than WhileLoopLt
    // but handles everything: `while a && b`, `while func()`, etc.
    pub fn compile_while_generic(&mut self, condition: &Expr, body: &Stmt) -> Result<()> {
        let loop_start = self.current_offset();

        self.loop_stack.push(LoopContext {
            start: loop_start,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_for_loop: false,
        });

        let cond_reg = self.alloc_register()?;
        self.compile_expr(condition, cond_reg)?;

        let jump_to_end = self.emit_jump_if(OpCode::JumpIfNot, cond_reg, condition.span);
        self.free_register(cond_reg);

        self.compile_stmt(body)?;

        let jump_dist = (self.current_offset() - loop_start + 1) as i16;
        self.emit_b(OpCode::Jump, 0, -jump_dist, body.span);

        self.patch_jump(jump_to_end);

        if let Some(loop_ctx) = self.loop_stack.pop() {
            for break_jump in loop_ctx.break_jumps {
                self.patch_jump(break_jump);
            }
        }

        Ok(())
    }
}

use super::super::{Compiler, LoopContext};
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;
use aelys_syntax::ast::{Expr, ExprKind, Stmt};

impl Compiler {
    // Inspired by Lua's FORPREP/FORLOOP super-instructions
    // The trick: subtract step BEFORE the loop starts, then ForLoopI always adds it back
    // This way we don't need separate "first iteration" logic
    //
    // Structure:
    //   1. iter = iter - step  (compensate for ForLoopI's increment)
    //   2. Jump forward to ForLoopI
    //   3. <body>
    //   4. ForLoopI: iter += step, check condition, jump back to body if true
    pub fn compile_for(
        &mut self,
        iterator: &str,
        start: &Expr,
        end: &Expr,
        inclusive: bool,
        step: Option<&Expr>,
        body: &Stmt,
        span: Span,
    ) -> Result<()> {
        // Try to figure out loop direction at compile time.
        // If we can't (e.g., `for i in a..b` where a,b are variables), we emit
        // runtime detection code which is slower but necessary.
        let compile_time_direction: Option<bool> = if let Some(step_expr) = step {
            if let ExprKind::Int(step_val) = &step_expr.kind {
                Some(*step_val > 0)
            } else {
                None // step is a variable, can't know at compile time
            }
        } else {
            match (&start.kind, &end.kind) {
                (ExprKind::Int(start_val), ExprKind::Int(end_val)) => Some(start_val <= end_val),
                _ => None,
            }
        };

        self.begin_scope();
        let iter_reg = self.declare_variable(iterator, false)?;
        self.loop_variables.push(iterator.to_string());

        let end_reg = iter_reg.checked_add(1).ok_or_else(|| {
            CompileError::new(
                CompileErrorKind::TooManyRegisters,
                span,
                self.source.clone(),
            )
        })?;
        let step_reg = iter_reg.checked_add(2).ok_or_else(|| {
            CompileError::new(
                CompileErrorKind::TooManyRegisters,
                span,
                self.source.clone(),
            )
        })?;

        self.alloc_consecutive_from(end_reg, 2)?; // end + step, must be adjacent

        self.compile_expr(start, iter_reg)?;
        self.compile_expr(end, end_reg)?;

        if let Some(step_expr) = step {
            self.compile_expr(step_expr, step_reg)?;
        } else if let Some(direction) = compile_time_direction {
            let step_val = if direction { 1i16 } else { -1i16 };
            self.emit_b(OpCode::LoadI, step_reg, step_val, span);
        } else {
            // Runtime direction detection - only used when we can't determine
            // direction at compile time. Costs a few extra instructions but
            // beats having the user specify step explicitly.
            let temp_reg = self.alloc_register()?;

            // if start > end: step = -1, else step = 1
            self.emit_a(OpCode::Gt, temp_reg, iter_reg, end_reg, span);

            let jump_to_pos = self.emit_jump_if(OpCode::JumpIfNot, temp_reg, span);
            self.emit_b(OpCode::LoadI, step_reg, -1, span);
            let jump_past = self.emit_jump(OpCode::Jump, span);

            self.patch_jump(jump_to_pos);
            self.emit_b(OpCode::LoadI, step_reg, 1, span);

            self.patch_jump(jump_past);
            self.free_register(temp_reg);
        }

        self.emit_a(OpCode::Sub, iter_reg, iter_reg, step_reg, span);
        let jump_to_forloop = self.emit_jump(OpCode::Jump, span);

        let body_start = self.current_offset();

        self.loop_stack.push(LoopContext {
            start: body_start,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_for_loop: true,
        });

        self.compile_stmt(body)?;

        let forloop_pos = self.current_offset();

        self.patch_jump(jump_to_forloop);

        // Patch continue statements to jump to ForLoopI instead of past the loop.
        // This is slightly ugly - we're manually reconstructing the instruction encoding.
        // TODO: maybe add a helper for patching just the offset part of a jump?
        if let Some(ctx) = self.loop_stack.last() {
            for &continue_jump in &ctx.continue_jumps {
                let dist = (forloop_pos - continue_jump - 1) as i16;
                let instr = self.current.bytecode_at(continue_jump);
                let op = instr >> 24;
                let a = (instr >> 16) & 0xFF;
                *self.current.bytecode_mut(continue_jump) =
                    (op << 24) | (a << 16) | ((dist as u16) as u32);
            }
        }

        let loop_opcode = if inclusive {
            OpCode::ForLoopIInc
        } else {
            OpCode::ForLoopI
        };
        let jump_back_dist = -((forloop_pos - body_start + 1) as i16);
        self.emit_b(loop_opcode, iter_reg, jump_back_dist, span);

        if let Some(loop_ctx) = self.loop_stack.pop() {
            for break_jump in loop_ctx.break_jumps {
                self.patch_jump(break_jump);
            }
        }

        self.free_register(step_reg);
        self.free_register(end_reg);
        self.loop_variables.pop();
        self.end_scope();

        Ok(())
    }
}

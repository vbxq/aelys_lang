use super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;

impl Compiler {
    pub fn compile_typed_while(
        &mut self,
        condition: &aelys_sema::TypedExpr,
        body: &aelys_sema::TypedStmt,
        span: Span,
    ) -> Result<()> {
        let loop_start = self.current_offset();

        self.loop_stack.push(super::super::LoopContext {
            start: loop_start,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_for_loop: false,
        });

        let cond_reg = self.alloc_register()?;
        self.compile_typed_expr(condition, cond_reg)?;

        let exit_jump = self.emit_jump_if(OpCode::JumpIfNot, cond_reg, span);
        self.free_register(cond_reg);

        self.compile_typed_stmt(body)?;

        let jump_dist = (self.current_offset() - loop_start + 1) as i16;
        self.emit_b(OpCode::Jump, 0, -jump_dist, span);

        self.patch_jump(exit_jump);

        let ctx = self.loop_stack.pop().ok_or_else(|| {
            CompileError::new(
                CompileErrorKind::BreakOutsideLoop,
                span,
                self.source.clone(),
            )
        })?;
        for jump in ctx.break_jumps {
            self.patch_jump(jump);
        }

        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn compile_typed_for(
        &mut self,
        iterator: &str,
        start: &aelys_sema::TypedExpr,
        end: &aelys_sema::TypedExpr,
        inclusive: bool,
        step: Option<&aelys_sema::TypedExpr>,
        body: &aelys_sema::TypedStmt,
        span: Span,
    ) -> Result<()> {
        self.begin_scope();

        let iter_reg = self.alloc_consecutive_registers_for_call(3, span)?;
        let end_reg = iter_reg + 1;
        let step_reg = iter_reg + 2;

        self.register_pool[iter_reg as usize] = true;
        self.register_pool[end_reg as usize] = true;
        self.register_pool[step_reg as usize] = true;
        self.next_register = self.next_register.max(step_reg + 1);

        self.compile_typed_expr(start, iter_reg)?;
        self.compile_typed_expr(end, end_reg)?;

        if let Some(step_expr) = step {
            self.compile_typed_expr(step_expr, step_reg)?;
        } else {
            self.emit_b(OpCode::LoadI, step_reg, 1, span);
        }

        self.add_local(
            iterator.to_string(),
            false,
            iter_reg,
            aelys_sema::ResolvedType::Int,
        );
        self.loop_variables.push(iterator.to_string());

        // Subtract step from iter so that the first ForLoopI increment gives the correct start value
        // This ensures that for empty ranges (like 0..0), the loop body is never executed
        self.emit_a(OpCode::Sub, iter_reg, iter_reg, step_reg, span);

        // Jump to the ForLoopI check before executing the body for the first time
        let jump_to_forloop = self.emit_jump(OpCode::Jump, span);

        let loop_start = self.current_offset();

        self.loop_stack.push(super::super::LoopContext {
            start: loop_start,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_for_loop: true,
        });

        let opcode = if inclusive {
            OpCode::ForLoopIInc
        } else {
            OpCode::ForLoopI
        };

        self.compile_typed_stmt(body)?;

        let continue_target = self.current_offset();

        // Patch the initial jump to point here (to ForLoopI)
        self.patch_jump(jump_to_forloop);

        let offset = (self.current_offset() - loop_start + 1) as i16;
        self.emit_b(opcode, iter_reg, -offset, span);

        let ctx = self.loop_stack.pop().ok_or_else(|| {
            CompileError::new(
                CompileErrorKind::ContinueOutsideLoop,
                span,
                self.source.clone(),
            )
        })?;
        for jump in ctx.continue_jumps {
            let offset_to_target = (continue_target as isize - jump as isize - 1) as i16;
            *self.current.bytecode_mut(jump) =
                (OpCode::Jump as u32) << 24 | ((offset_to_target as u32) & 0xFFFFFF);
        }

        for jump in ctx.break_jumps {
            self.patch_jump(jump);
        }

        self.loop_variables.pop();
        self.free_register(step_reg);
        self.free_register(end_reg);
        self.end_scope();

        Ok(())
    }
}

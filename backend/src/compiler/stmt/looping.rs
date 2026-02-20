use super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_sema::InferType;
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
            aelys_sema::ResolvedType::I64,
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

    pub fn compile_typed_for_each(
        &mut self,
        iterator: &str,
        iterable: &aelys_sema::TypedExpr,
        elem_type: &InferType,
        body: &aelys_sema::TypedStmt,
        span: Span,
    ) -> Result<()> {
        match &iterable.ty {
            InferType::String => self.compile_string_for_each(iterator, iterable, body, span),
            InferType::Vec(inner) => self.compile_collection_for_each(
                iterator,
                iterable,
                inner,
                body,
                OpCode::VecForLoop,
                span,
            ),
            InferType::Array(inner) => self.compile_collection_for_each(
                iterator,
                iterable,
                inner,
                body,
                OpCode::ArrayForLoop,
                span,
            ),
            InferType::Dynamic | InferType::Var(_) => {
                // dynamic: default to VecForLoop (works for both at runtime via object kind)
                self.compile_collection_for_each(
                    iterator,
                    iterable,
                    &InferType::Dynamic,
                    body,
                    OpCode::VecForLoop,
                    span,
                )
            }
            _ => Err(aelys_common::AelysError::Compile(CompileError::new(
                CompileErrorKind::TypeInferenceError(format!(
                    "for-each over {:?} not yet supported",
                    elem_type
                )),
                span,
                self.source.clone(),
            ))),
        }
    }

    fn infer_to_resolved(ty: &InferType) -> aelys_sema::ResolvedType {
        aelys_sema::ResolvedType::from_infer_type(ty)
    }

    fn compile_collection_for_each(
        &mut self,
        iterator: &str,
        iterable: &aelys_sema::TypedExpr,
        inner_type: &InferType,
        body: &aelys_sema::TypedStmt,
        opcode: OpCode,
        span: Span,
    ) -> Result<()> {
        self.begin_scope();

        // Allocate 3 consecutive registers: [element, index, collection_ptr]
        let elem_reg = self.alloc_consecutive_registers_for_call(3, span)?;
        let index_reg = elem_reg + 1;
        let coll_reg = elem_reg + 2;

        self.register_pool[elem_reg as usize] = true;
        self.register_pool[index_reg as usize] = true;
        self.register_pool[coll_reg as usize] = true;
        self.next_register = self.next_register.max(coll_reg + 1);

        // Compile iterable into collection_ptr register
        self.compile_typed_expr(iterable, coll_reg)?;

        // Initialize index to 0
        self.emit_b(OpCode::LoadI, index_reg, 0, span);

        // Jump to ForLoop check before executing body
        let jump_to_forloop = self.emit_jump(OpCode::Jump, span);

        let loop_start = self.current_offset();

        self.loop_stack.push(super::super::LoopContext {
            start: loop_start,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_for_loop: true,
        });

        // Register the iterator variable pointing to element register
        let resolved = Self::infer_to_resolved(inner_type);
        self.add_local(iterator.to_string(), false, elem_reg, resolved);

        // Compile loop body
        self.compile_typed_stmt(body)?;

        let continue_target = self.current_offset();

        // Patch the initial jump to point here (to VecForLoop/ArrayForLoop)
        self.patch_jump(jump_to_forloop);

        // Emit VecForLoop/ArrayForLoop: operates on elem_reg (consecutive regs)
        let offset = (self.current_offset() - loop_start + 1) as i16;
        self.emit_b(opcode, elem_reg, -offset, span);

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

        self.free_register(coll_reg);
        self.free_register(index_reg);
        self.end_scope();

        Ok(())
    }

    fn compile_string_for_each(
        &mut self,
        iterator: &str,
        iterable: &aelys_sema::TypedExpr,
        body: &aelys_sema::TypedStmt,
        span: Span,
    ) -> Result<()> {
        self.begin_scope();

        // Allocate 3 consecutive registers: [char_result, byte_offset, string_ptr]
        let char_reg = self.alloc_consecutive_registers_for_call(3, span)?;
        let offset_reg = char_reg + 1;
        let str_reg = char_reg + 2;

        self.register_pool[char_reg as usize] = true;
        self.register_pool[offset_reg as usize] = true;
        self.register_pool[str_reg as usize] = true;
        self.next_register = self.next_register.max(str_reg + 1);

        // Compile iterable into string_ptr register
        self.compile_typed_expr(iterable, str_reg)?;

        // Initialize byte_offset to 0
        self.emit_b(OpCode::LoadI, offset_reg, 0, span);

        // Jump to StringForLoop check before executing body
        let jump_to_forloop = self.emit_jump(OpCode::Jump, span);

        let loop_start = self.current_offset();

        self.loop_stack.push(super::super::LoopContext {
            start: loop_start,
            break_jumps: Vec::new(),
            continue_jumps: Vec::new(),
            is_for_loop: true,
        });

        // Register the iterator variable pointing to char_result register
        self.add_local(
            iterator.to_string(),
            false,
            char_reg,
            aelys_sema::ResolvedType::String,
        );

        // Compile loop body
        self.compile_typed_stmt(body)?;

        let continue_target = self.current_offset();

        // Patch the initial jump to point here (to StringForLoop)
        self.patch_jump(jump_to_forloop);

        // Emit StringForLoop: operates on char_reg (consecutive regs)
        let offset = (self.current_offset() - loop_start + 1) as i16;
        self.emit_b(OpCode::StringForLoop, char_reg, -offset, span);

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

        self.free_register(str_reg);
        self.free_register(offset_reg);
        self.end_scope();

        Ok(())
    }
}

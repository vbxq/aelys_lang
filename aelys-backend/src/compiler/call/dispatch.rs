use super::super::Compiler;
use super::util::arg_range_available;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;
use aelys_syntax::ast::{Expr, ExprKind};

impl Compiler {
    // Call dispatch: try specialized opcodes first, fall back to generic Call.
    // Order matters - upvalue calls are fastest (no lookup), then module (cached),
    // then global (cached), then generic (full lookup every time).
    pub fn compile_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        dest: u8,
        span: Span,
    ) -> Result<()> {
        if args.len() > 255 {
            return Err(CompileError::new(
                CompileErrorKind::TooManyArguments,
                span,
                self.source.clone(),
            )
            .into());
        }

        // each of these returns true if it handled the call
        // builtins first - fastest path for alloc/free/load/store
        if self.try_compile_builtin_call(callee, args, dest, span)? {
            return Ok(());
        }

        if self.try_compile_upvalue_call(callee, args, dest, span)? {
            return Ok(());
        }

        if self.try_compile_module_call(callee, args, dest, span)? {
            return Ok(());
        }

        if self.try_compile_global_call(callee, args, dest, span)? {
            return Ok(());
        }

        // nothing matched - use the slow path
        self.compile_call_generic(callee, args, dest, span)
    }

    pub(super) fn reserve_arg_registers(&mut self, start: u8, args_len: usize) -> bool {
        if !arg_range_available(&self.register_pool, start, args_len) {
            return false;
        }
        for i in 0..args_len {
            let arg_reg = start + i as u8;
            self.register_pool[arg_reg as usize] = true;
            if arg_reg >= self.next_register {
                self.next_register = arg_reg + 1;
            }
        }
        true
    }

    pub(super) fn release_arg_registers(&mut self, start: u8, args_len: usize) {
        for i in (0..args_len).rev() {
            let arg_reg = start + i as u8;
            self.register_pool[arg_reg as usize] = false;
        }
    }

    pub(super) fn checked_arg_start(&self, dest: u8) -> Option<u8> {
        dest.checked_add(1)
    }

    pub(super) fn is_member_call(callee: &Expr) -> Option<(&str, &str)> {
        if let ExprKind::Member { object, member } = &callee.kind {
            if let ExprKind::Identifier(module_name) = &object.kind {
                return Some((module_name, member));
            }
        }
        None
    }
}

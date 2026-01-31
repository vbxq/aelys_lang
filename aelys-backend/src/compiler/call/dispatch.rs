use super::super::Compiler;
use super::util::arg_range_available;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Span;
use aelys_syntax::ast::{Expr, ExprKind, FmtStringPart};

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

        // Handle format string with placeholders: func("x={}", x) -> func("x=" + __tostring(x))
        if let Some((fmt_parts, placeholder_count)) = Self::get_fmt_string_placeholders(args) {
            if placeholder_count > 0 {
                return self.compile_call_with_fmt_placeholders(
                    callee, args, fmt_parts, placeholder_count, dest, span
                );
            }
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

    fn get_fmt_string_placeholders(args: &[Expr]) -> Option<(&[FmtStringPart], usize)> {
        if args.is_empty() {
            return None;
        }
        if let ExprKind::FmtString(parts) = &args[0].kind {
            let count = parts.iter().filter(|p| matches!(p, FmtStringPart::Placeholder)).count();
            return Some((parts, count));
        }
        None
    }

    fn compile_call_with_fmt_placeholders(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        fmt_parts: &[FmtStringPart],
        placeholder_count: usize,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let extra_args_needed = placeholder_count;
        let extra_args_available = args.len() - 1;

        if extra_args_available < extra_args_needed {
            return Err(CompileError::new(
                CompileErrorKind::TypeInferenceError(format!(
                    "format string has {} placeholder(s) but only {} argument(s) provided",
                    extra_args_needed, extra_args_available
                )),
                span,
                self.source.clone(),
            ).into());
        }

        let fmt_extra_args = &args[1..1 + extra_args_needed];
        let remaining_args = &args[1 + extra_args_needed..];

        // Compile: func(fmt_string_expanded, remaining_args...)
        let total_args = 1 + remaining_args.len();
        let func_reg = self.alloc_consecutive_registers_for_call(total_args as u8 + 1, span)?;

        for i in 0..=total_args {
            let reg = func_reg + i as u8;
            self.register_pool[reg as usize] = true;
            if reg >= self.next_register {
                self.next_register = reg + 1;
            }
        }

        self.compile_expr(callee, func_reg)?;

        // first arg: the expanded format string
        let fmt_reg = func_reg + 1;
        self.compile_fmt_string(fmt_parts, fmt_extra_args, fmt_reg, args[0].span)?;

        // remaining args
        for (i, arg) in remaining_args.iter().enumerate() {
            let arg_reg = func_reg + 2 + i as u8;
            self.compile_expr(arg, arg_reg)?;
        }

        self.emit_c(aelys_bytecode::OpCode::Call, dest, func_reg, total_args as u8, span);

        for i in (0..=total_args).rev() {
            let reg = func_reg + i as u8;
            self.register_pool[reg as usize] = false;
        }

        Ok(())
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

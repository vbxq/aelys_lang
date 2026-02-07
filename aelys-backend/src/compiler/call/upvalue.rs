use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::{Expr, ExprKind};

impl Compiler {
    // CallUpval: for calling functions captured from outer scopes.
    // this is the fast path for recursive closures, the function is already in the upvalue array, no name lookup needed at runtime.
    pub(super) fn try_compile_upvalue_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        dest: u8,
        span: Span,
    ) -> Result<bool> {
        if let ExprKind::Identifier(name) = &callee.kind {
            // Not a local? Check if it's captured from an enclosing scope
            if self.resolve_variable(name).is_none()
                && let Some((upval_idx, _mutable)) = self.resolve_upvalue(name)
            {
                let arg_start = match self.checked_arg_start(dest) {
                    Some(s) => s,
                    None => {
                        self.compile_call_generic(callee, args, dest, span)?;
                        return Ok(true);
                    }
                };

                if !self.reserve_arg_registers(arg_start, args.len()) {
                    self.compile_call_generic(callee, args, dest, span)?;
                    return Ok(true);
                }

                for (i, arg) in args.iter().enumerate() {
                    let arg_reg = arg_start + i as u8;
                    self.compile_expr(arg, arg_reg)?;
                }

                self.emit_a(OpCode::CallUpval, dest, upval_idx, args.len() as u8, span);
                self.release_arg_registers(arg_start, args.len());
                return Ok(true);
            }
        }

        Ok(false)
    }
}

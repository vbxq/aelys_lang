use super::super::Compiler;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::Expr;

impl Compiler {
    pub(super) fn try_compile_module_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        dest: u8,
        span: Span,
    ) -> Result<bool> {
        if let Some((module_name, member)) = Self::is_member_call(callee)
            && self.module_aliases.contains(module_name)
        {
            let qualified_name = format!("{}::{}", module_name, member);
            let global_idx = self.get_or_create_global_index(&qualified_name);
            self.accessed_globals.insert(qualified_name.clone());

            if global_idx <= 255 {
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

                self.emit_call_global_cached(
                    dest,
                    global_idx as u8,
                    args.len() as u8,
                    &qualified_name,
                    span,
                );
                self.release_arg_registers(arg_start, args.len());
                return Ok(true);
            }

            self.compile_call_generic(callee, args, dest, span)?;
            return Ok(true);
        }

        Ok(false)
    }
}

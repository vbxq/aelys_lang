use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;

impl Compiler {
    pub(super) fn compile_typed_call(
        &mut self,
        callee: &aelys_sema::TypedExpr,
        args: &[aelys_sema::TypedExpr],
        dest: u8,
        span: Span,
    ) -> Result<()> {
        use aelys_sema::TypedExprKind;

        if let TypedExprKind::Member { object, member } = &callee.kind {
            if let TypedExprKind::Identifier(module_name) = &object.kind {
                if self.module_aliases.contains(module_name) {
                    let qualified_name = format!("{}::{}", module_name, member);
                    let global_idx = self.get_or_create_global_index(&qualified_name);
                    self.accessed_globals.insert(qualified_name.clone());

                    if global_idx <= 255 {
                        let arg_start = match dest.checked_add(1) {
                            Some(s) => s,
                            None => {
                                return self.compile_typed_call_fallback(callee, args, dest, span);
                            }
                        };

                        let mut can_use_callglobal = true;
                        for i in 0..args.len() {
                            let arg_reg = match arg_start.checked_add(i as u8) {
                                Some(r) => r,
                                None => {
                                    can_use_callglobal = false;
                                    break;
                                }
                            };
                            if (arg_reg as usize) >= self.register_pool.len()
                                || self.register_pool[arg_reg as usize]
                            {
                                can_use_callglobal = false;
                                break;
                            }
                        }

                        if can_use_callglobal {
                            for i in 0..args.len() {
                                let arg_reg = arg_start + i as u8;
                                self.register_pool[arg_reg as usize] = true;
                                if arg_reg >= self.next_register {
                                    self.next_register = arg_reg + 1;
                                }
                            }

                            for (i, arg) in args.iter().enumerate() {
                                let arg_reg = arg_start + i as u8;
                                self.compile_typed_expr(arg, arg_reg)?;
                            }

                            self.emit_call_global_cached(
                                dest,
                                global_idx as u8,
                                args.len() as u8,
                                &qualified_name,
                                span,
                            );

                            for i in (0..args.len()).rev() {
                                let arg_reg = arg_start + i as u8;
                                self.register_pool[arg_reg as usize] = false;
                            }

                            return Ok(());
                        }
                    }
                }
            }
        }

        if let TypedExprKind::Identifier(name) = &callee.kind {
            if Self::is_builtin(name) {
                return self.compile_typed_builtin_call(name, args, dest, span);
            }

            if self.resolve_variable(name).is_none() && self.resolve_upvalue(name).is_none() {
                let global_idx = self.get_or_create_global_index(name);
                self.accessed_globals.insert(name.to_string());

                if global_idx <= 255 {
                    let arg_start = match dest.checked_add(1) {
                        Some(s) => s,
                        None => return self.compile_typed_call_fallback(callee, args, dest, span),
                    };

                    let mut can_use_callglobal = true;
                    for i in 0..args.len() {
                        let arg_reg = match arg_start.checked_add(i as u8) {
                            Some(r) => r,
                            None => {
                                can_use_callglobal = false;
                                break;
                            }
                        };
                        if (arg_reg as usize) >= self.register_pool.len()
                            || self.register_pool[arg_reg as usize]
                        {
                            can_use_callglobal = false;
                            break;
                        }
                    }

                    if can_use_callglobal {
                        for i in 0..args.len() {
                            let arg_reg = arg_start + i as u8;
                            self.register_pool[arg_reg as usize] = true;
                            if arg_reg >= self.next_register {
                                self.next_register = arg_reg + 1;
                            }
                        }

                        for (i, arg) in args.iter().enumerate() {
                            let arg_reg = arg_start + i as u8;
                            self.compile_typed_expr(arg, arg_reg)?;
                        }

                        self.emit_call_global_cached(
                            dest,
                            global_idx as u8,
                            args.len() as u8,
                            name,
                            span,
                        );

                        for i in (0..args.len()).rev() {
                            let arg_reg = arg_start + i as u8;
                            self.register_pool[arg_reg as usize] = false;
                        }

                        return Ok(());
                    }
                }
            }
        }

        self.compile_typed_call_fallback(callee, args, dest, span)
    }

    pub(super) fn compile_typed_call_fallback(
        &mut self,
        callee: &aelys_sema::TypedExpr,
        args: &[aelys_sema::TypedExpr],
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let nargs = args.len();
        let callee_reg = self.alloc_consecutive_registers_for_call(nargs as u8 + 1, span)?;

        for i in 0..=nargs {
            let reg = callee_reg + i as u8;
            self.register_pool[reg as usize] = true;
            if reg >= self.next_register {
                self.next_register = reg + 1;
            }
        }

        self.compile_typed_expr(callee, callee_reg)?;

        for (i, arg) in args.iter().enumerate() {
            let arg_reg = callee_reg + 1 + i as u8;
            self.compile_typed_expr(arg, arg_reg)?;
        }

        self.emit_a(OpCode::Call, dest, callee_reg, args.len() as u8, span);

        for i in (0..=nargs).rev() {
            let reg = callee_reg + i as u8;
            self.register_pool[reg as usize] = false;
        }

        Ok(())
    }
}

use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_sema::{InferType, TypedFmtStringPart};
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

        // Handle format string with placeholders: func("x={}", x) -> func("x=" + __tostring(x))
        if let Some((fmt_parts, placeholder_count)) = Self::get_typed_fmt_placeholders(args)
            && placeholder_count > 0
        {
            return self.compile_typed_call_with_fmt_placeholders(
                callee,
                args,
                fmt_parts,
                placeholder_count,
                dest,
                span,
            );
        }

        // Check for Array/Vec method calls first
        if let TypedExprKind::Member { object, member } = &callee.kind {
            // Handle Array methods
            if let InferType::Array(_) = &object.ty
                && member == "len"
                && args.is_empty()
            {
                return self.compile_array_len(object, dest, span);
            }

            // Handle Vec methods
            if let InferType::Vec(inner) = &object.ty {
                match member.as_str() {
                    "len" if args.is_empty() => {
                        return self.compile_vec_len(object, dest, span);
                    }
                    "push" if args.len() == 1 => {
                        return self.compile_vec_push(object, inner, &args[0], dest, span);
                    }
                    "pop" if args.is_empty() => {
                        return self.compile_vec_pop(object, inner, dest, span);
                    }
                    "capacity" if args.is_empty() => {
                        return self.compile_vec_capacity(object, dest, span);
                    }
                    "reserve" if args.len() == 1 => {
                        return self.compile_vec_reserve(object, &args[0], dest, span);
                    }
                    _ => {}
                }
            }

            // Handle String methods: s.method(args) → string::method(s, args...)
            if matches!(&object.ty, InferType::String) {
                if let Some(expected_args) = Self::string_method_arity(member) {
                    if args.len() == expected_args {
                        return self.compile_string_method_call(
                            object, member, args, dest, span,
                        );
                    }
                }
            }

            // Handle to_string() on any type
            if member == "to_string" && args.is_empty() {
                return self.compile_tostring_method(object, dest, span);
            }

            // Handle Vec/Array methods on Dynamic-typed objects (runtime dispatch)
            if matches!(&object.ty, InferType::Dynamic | InferType::Var(_)) {
                // Try string methods on dynamic types too
                if let Some(expected_args) = Self::string_method_arity(member) {
                    if args.len() == expected_args {
                        return self.compile_string_method_call(
                            object, member, args, dest, span,
                        );
                    }
                }

                match member.as_str() {
                    "len" if args.is_empty() => {
                        return self.compile_vec_len(object, dest, span);
                    }
                    "push" if args.len() == 1 => {
                        return self.compile_vec_push(
                            object,
                            &InferType::Dynamic,
                            &args[0],
                            dest,
                            span,
                        );
                    }
                    "pop" if args.is_empty() => {
                        return self.compile_vec_pop(object, &InferType::Dynamic, dest, span);
                    }
                    "capacity" if args.is_empty() => {
                        return self.compile_vec_capacity(object, dest, span);
                    }
                    "reserve" if args.len() == 1 => {
                        return self.compile_vec_reserve(object, &args[0], dest, span);
                    }
                    _ => {}
                }
            }

            if let TypedExprKind::Identifier(module_name) = &object.kind
                && self.module_aliases.contains(module_name)
            {
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

        if let TypedExprKind::Identifier(name) = &callee.kind {
            if Self::is_builtin(name) {
                return self.compile_typed_builtin_call(name, args, dest, span);
            }

            if self.resolve_variable(name).is_none() && self.resolve_upvalue(name).is_none() {
                if !self.globals.contains_key(name) && !self.known_globals.contains(name) {
                    return self.compile_typed_call_fallback(callee, args, dest, span);
                }
                let actual_name = self.resolve_global_name(name).to_string();
                let global_idx = self.get_or_create_global_index(name);
                self.accessed_globals.insert(actual_name.clone());

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

    /// returns the number of extra args (excluding self) expected by a string method, or None if not a valid method.
    fn string_method_arity(method: &str) -> Option<usize> {
        match method {
            // 0-arg methods (only self)
            "len" | "char_len" | "chars" | "bytes" | "to_upper" | "to_lower"
            | "capitalize" | "trim" | "trim_start" | "trim_end" | "is_empty"
            | "is_whitespace" | "is_numeric" | "is_alphabetic"
            | "is_alphanumeric" | "reverse" | "lines" | "line_count" => Some(0),
            // 1-arg methods (self + 1 arg)
            "char_at" | "byte_at" | "contains" | "starts_with" | "ends_with"
            | "find" | "rfind" | "count" | "split" | "repeat" | "concat" => Some(1),
            // 2-arg methods (self + 2 args)
            "substr" | "replace" | "replace_first" | "join" | "pad_left" | "pad_right" => Some(2),
            _ => None,
        }
    }

    /// compile s.method(args) as string::method(s, args...)
    fn compile_string_method_call(
        &mut self,
        object: &aelys_sema::TypedExpr,
        method: &str,
        args: &[aelys_sema::TypedExpr],
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let qualified_name = format!("string::{}", method);
        let total_args = 1 + args.len(); // self + extra args

        let global_idx = self.get_or_create_global_index(&qualified_name);
        self.accessed_globals.insert(qualified_name.clone());

        if global_idx <= 255 {
            // Try to use CallGlobalCached: args go in dest+1, dest+2, ...
            let arg_start = match dest.checked_add(1) {
                Some(s) => s,
                None => {
                    return self.compile_string_method_call_fallback(
                        object, &qualified_name, args, dest, span,
                    );
                }
            };

            let mut can_use_callglobal = true;
            for i in 0..total_args {
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
                // Reserve registers
                for i in 0..total_args {
                    let arg_reg = arg_start + i as u8;
                    self.register_pool[arg_reg as usize] = true;
                    if arg_reg >= self.next_register {
                        self.next_register = arg_reg + 1;
                    }
                }

                // First arg: the string object itself
                self.compile_typed_expr(object, arg_start)?;

                // Remaining args
                for (i, arg) in args.iter().enumerate() {
                    let arg_reg = arg_start + 1 + i as u8;
                    self.compile_typed_expr(arg, arg_reg)?;
                }

                self.emit_call_global_cached(
                    dest,
                    global_idx as u8,
                    total_args as u8,
                    &qualified_name,
                    span,
                );

                // Free registers
                for i in (0..total_args).rev() {
                    let arg_reg = arg_start + i as u8;
                    self.register_pool[arg_reg as usize] = false;
                }

                return Ok(());
            }
        }

        self.compile_string_method_call_fallback(object, &qualified_name, args, dest, span)
    }

    fn compile_string_method_call_fallback(
        &mut self,
        object: &aelys_sema::TypedExpr,
        qualified_name: &str,
        args: &[aelys_sema::TypedExpr],
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let total_args = 1 + args.len();
        let callee_reg = self.alloc_consecutive_registers_for_call(total_args as u8 + 1, span)?;

        for i in 0..=total_args {
            let reg = callee_reg + i as u8;
            self.register_pool[reg as usize] = true;
            if reg >= self.next_register {
                self.next_register = reg + 1;
            }
        }

        // load the function by global index (avoids known_globals check)
        let global_idx = self.get_or_create_global_index(qualified_name);
        self.accessed_globals.insert(qualified_name.to_string());
        self.emit_b(OpCode::GetGlobalIdx, callee_reg, global_idx as i16, span);

        // first arg: the string object
        self.compile_typed_expr(object, callee_reg + 1)?;

        // remaining stuff
        for (i, arg) in args.iter().enumerate() {
            let arg_reg = callee_reg + 2 + i as u8;
            self.compile_typed_expr(arg, arg_reg)?;
        }

        self.emit_a(OpCode::Call, dest, callee_reg, total_args as u8, span);

        for i in (0..=total_args).rev() {
            let reg = callee_reg + i as u8;
            self.register_pool[reg as usize] = false;
        }

        Ok(())
    }

    /// compiled obj.to_string() as __tostring(obj)
    fn compile_tostring_method(
        &mut self,
        object: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let qualified_name = "__tostring";
        let global_idx = self.get_or_create_global_index(qualified_name);
        self.accessed_globals.insert(qualified_name.to_string());

        if global_idx <= 255 {
            let arg_start = match dest.checked_add(1) {
                Some(s) if (s as usize) < self.register_pool.len()
                    && !self.register_pool[s as usize] =>
                {
                    s
                }
                _ => {
                    return self.compile_tostring_method_fallback(object, dest, span);
                }
            };

            self.register_pool[arg_start as usize] = true;
            if arg_start >= self.next_register {
                self.next_register = arg_start + 1;
            }

            self.compile_typed_expr(object, arg_start)?;

            self.emit_call_global_cached(
                dest,
                global_idx as u8,
                1,
                qualified_name,
                span,
            );

            self.register_pool[arg_start as usize] = false;
            return Ok(());
        }

        self.compile_tostring_method_fallback(object, dest, span)
    }

    fn compile_tostring_method_fallback(
        &mut self,
        object: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let callee_reg = self.alloc_consecutive_registers_for_call(2, span)?;

        self.register_pool[callee_reg as usize] = true;
        self.register_pool[(callee_reg + 1) as usize] = true;
        if callee_reg + 1 >= self.next_register {
            self.next_register = callee_reg + 2;
        }

        let global_idx = self.get_or_create_global_index("__tostring");
        self.accessed_globals.insert("__tostring".to_string());
        self.emit_b(OpCode::GetGlobalIdx, callee_reg, global_idx as i16, span);
        self.compile_typed_expr(object, callee_reg + 1)?;

        self.emit_a(OpCode::Call, dest, callee_reg, 1, span);

        self.register_pool[(callee_reg + 1) as usize] = false;
        self.register_pool[callee_reg as usize] = false;

        Ok(())
    }

    fn get_typed_fmt_placeholders(
        args: &[aelys_sema::TypedExpr],
    ) -> Option<(&[TypedFmtStringPart], usize)> {
        use aelys_sema::TypedExprKind;
        if args.is_empty() {
            return None;
        }
        if let TypedExprKind::FmtString(parts) = &args[0].kind {
            let count = parts
                .iter()
                .filter(|p| matches!(p, TypedFmtStringPart::Placeholder))
                .count();
            return Some((parts, count));
        }
        None
    }

    fn compile_typed_call_with_fmt_placeholders(
        &mut self,
        callee: &aelys_sema::TypedExpr,
        args: &[aelys_sema::TypedExpr],
        fmt_parts: &[TypedFmtStringPart],
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
            )
            .into());
        }

        let fmt_extra_args = &args[1..1 + extra_args_needed];
        let remaining_args = &args[1 + extra_args_needed..];

        let total_args = 1 + remaining_args.len();
        let func_reg = self.alloc_consecutive_registers_for_call(total_args as u8 + 1, span)?;

        for i in 0..=total_args {
            let reg = func_reg + i as u8;
            self.register_pool[reg as usize] = true;
            if reg >= self.next_register {
                self.next_register = reg + 1;
            }
        }

        self.compile_typed_expr(callee, func_reg)?;

        let fmt_reg = func_reg + 1;
        self.compile_typed_fmt_string(fmt_parts, fmt_extra_args, fmt_reg, args[0].span)?;

        for (i, arg) in remaining_args.iter().enumerate() {
            let arg_reg = func_reg + 2 + i as u8;
            self.compile_typed_expr(arg, arg_reg)?;
        }

        self.emit_a(OpCode::Call, dest, func_reg, total_args as u8, span);

        for i in (0..=total_args).rev() {
            let reg = func_reg + i as u8;
            self.register_pool[reg as usize] = false;
        }

        Ok(())
    }
}

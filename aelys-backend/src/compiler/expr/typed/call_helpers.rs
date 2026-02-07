use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_sema::TypedExprKind;
use aelys_syntax::Span;

impl Compiler {
    pub(super) fn compile_typed_builtin_call(
        &mut self,
        name: &str,
        args: &[aelys_sema::TypedExpr],
        dest: u8,
        span: Span,
    ) -> Result<()> {
        // fast path: direct opcodes for memory operations
        match name {
            "alloc" if args.len() == 1 => {
                return self.compile_typed_alloc(&args[0], dest, span);
            }
            "free" if args.len() == 1 => {
                return self.compile_typed_free(&args[0], dest, span);
            }
            "load" if args.len() == 2 => {
                return self.compile_typed_load(&args[0], &args[1], dest, span);
            }
            "store" if args.len() == 3 => {
                return self.compile_typed_store(&args[0], &args[1], &args[2], dest, span);
            }
            _ => {}
        }

        // fallback to CallGlobalNative for 'type' and other builtins
        let idx = self.get_or_create_global_index(name);
        self.accessed_globals.insert(name.to_string());

        if idx > 255 {
            return self.compile_typed_call_generic(name, args, dest, span);
        }

        let arg_start = match dest.checked_add(1) {
            Some(s) => s,
            None => return self.compile_typed_call_generic(name, args, dest, span),
        };

        for i in 0..args.len() {
            let arg_reg = match arg_start.checked_add(i as u8) {
                Some(r) => r,
                None => return self.compile_typed_call_generic(name, args, dest, span),
            };
            if (arg_reg as usize) >= self.register_pool.len() {
                return self.compile_typed_call_generic(name, args, dest, span);
            }
            if self.register_pool[arg_reg as usize] {
                return self.compile_typed_call_generic(name, args, dest, span);
            }
        }

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

        self.emit_call_global_cached(dest, idx as u8, args.len() as u8, name, span);

        for i in (0..args.len()).rev() {
            let arg_reg = arg_start + i as u8;
            self.register_pool[arg_reg as usize] = false;
        }

        Ok(())
    }

    fn compile_typed_alloc(
        &mut self,
        size_expr: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let size_reg = self.alloc_register()?;
        self.compile_typed_expr(size_expr, size_reg)?;
        self.emit_a(OpCode::Alloc, dest, size_reg, 0, span);
        self.free_register(size_reg);
        Ok(())
    }

    fn compile_typed_free(
        &mut self,
        ptr_expr: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let ptr_reg = self.alloc_register()?;
        self.compile_typed_expr(ptr_expr, ptr_reg)?;
        self.emit_a(OpCode::Free, ptr_reg, 0, 0, span);
        self.free_register(ptr_reg);
        self.emit_a(OpCode::LoadNull, dest, 0, 0, span);
        Ok(())
    }

    fn compile_typed_load(
        &mut self,
        ptr_expr: &aelys_sema::TypedExpr,
        offset_expr: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        // Optimize: use LoadMemI for constant offsets 0-255
        if let TypedExprKind::Int(offset) = &offset_expr.kind
            && *offset >= 0
            && *offset <= 255
        {
            let ptr_reg = self.alloc_register()?;
            self.compile_typed_expr(ptr_expr, ptr_reg)?;
            self.emit_a(OpCode::LoadMemI, dest, ptr_reg, *offset as u8, span);
            self.free_register(ptr_reg);
            return Ok(());
        }

        let ptr_reg = self.alloc_register()?;
        let offset_reg = self.alloc_register()?;
        self.compile_typed_expr(ptr_expr, ptr_reg)?;
        self.compile_typed_expr(offset_expr, offset_reg)?;
        self.emit_a(OpCode::LoadMem, dest, ptr_reg, offset_reg, span);
        self.free_register(offset_reg);
        self.free_register(ptr_reg);
        Ok(())
    }

    fn compile_typed_store(
        &mut self,
        ptr_expr: &aelys_sema::TypedExpr,
        offset_expr: &aelys_sema::TypedExpr,
        value_expr: &aelys_sema::TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        // Optimize: use StoreMemI for constant offsets 0-255
        if let TypedExprKind::Int(offset) = &offset_expr.kind
            && *offset >= 0
            && *offset <= 255
        {
            let ptr_reg = self.alloc_register()?;
            let val_reg = self.alloc_register()?;
            self.compile_typed_expr(ptr_expr, ptr_reg)?;
            self.compile_typed_expr(value_expr, val_reg)?;
            self.emit_a(OpCode::StoreMemI, ptr_reg, *offset as u8, val_reg, span);
            self.free_register(val_reg);
            self.free_register(ptr_reg);
            self.emit_a(OpCode::LoadNull, dest, 0, 0, span);
            return Ok(());
        }

        let ptr_reg = self.alloc_register()?;
        let offset_reg = self.alloc_register()?;
        let val_reg = self.alloc_register()?;
        self.compile_typed_expr(ptr_expr, ptr_reg)?;
        self.compile_typed_expr(offset_expr, offset_reg)?;
        self.compile_typed_expr(value_expr, val_reg)?;
        self.emit_a(OpCode::StoreMem, ptr_reg, offset_reg, val_reg, span);
        self.free_register(val_reg);
        self.free_register(offset_reg);
        self.free_register(ptr_reg);
        self.emit_a(OpCode::LoadNull, dest, 0, 0, span);
        Ok(())
    }

    pub(super) fn compile_typed_call_generic(
        &mut self,
        name: &str,
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

        self.compile_identifier(name, callee_reg, span)?;

        for (i, arg) in args.iter().enumerate() {
            let arg_reg = callee_reg + 1 + i as u8;
            self.compile_typed_expr(arg, arg_reg)?;
        }

        self.emit_c(OpCode::Call, dest, callee_reg, args.len() as u8, span);

        for i in (0..=nargs).rev() {
            let reg = callee_reg + i as u8;
            self.register_pool[reg as usize] = false;
        }

        Ok(())
    }
}

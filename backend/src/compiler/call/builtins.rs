use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_syntax::Span;
use aelys_syntax::ast::{Expr, ExprKind};

impl Compiler {
    pub fn try_compile_builtin_call(
        &mut self,
        callee: &Expr,
        args: &[Expr],
        dest: u8,
        span: Span,
    ) -> Result<bool> {
        let name = match &callee.kind {
            ExprKind::Identifier(name) => name.as_str(),
            _ => return Ok(false),
        };

        match name {
            "alloc" => {
                if args.len() != 1 {
                    return Ok(false);
                }
                self.compile_builtin_alloc(&args[0], dest, span)?;
                Ok(true)
            }
            "free" => {
                if args.len() != 1 {
                    return Ok(false);
                }
                self.compile_builtin_free(&args[0], dest, span)?;
                Ok(true)
            }
            "load" => {
                if args.len() != 2 {
                    return Ok(false);
                }
                self.compile_builtin_load(&args[0], &args[1], dest, span)?;
                Ok(true)
            }
            "store" => {
                if args.len() != 3 {
                    return Ok(false);
                }
                self.compile_builtin_store(&args[0], &args[1], &args[2], dest, span)?;
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    fn compile_builtin_alloc(&mut self, size_expr: &Expr, dest: u8, span: Span) -> Result<()> {
        let size_reg = self.alloc_register()?;
        self.compile_expr(size_expr, size_reg)?;
        self.emit_a(OpCode::Alloc, dest, size_reg, 0, span);
        self.free_register(size_reg);
        Ok(())
    }

    fn compile_builtin_free(&mut self, ptr_expr: &Expr, dest: u8, span: Span) -> Result<()> {
        let ptr_reg = self.alloc_register()?;
        self.compile_expr(ptr_expr, ptr_reg)?;
        self.emit_a(OpCode::Free, ptr_reg, 0, 0, span);
        self.free_register(ptr_reg);
        self.emit_a(OpCode::LoadNull, dest, 0, 0, span);
        Ok(())
    }

    fn compile_builtin_load(
        &mut self,
        ptr_expr: &Expr,
        offset_expr: &Expr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        if let ExprKind::Int(offset) = &offset_expr.kind
            && *offset >= 0
            && *offset <= 255
        {
            let ptr_reg = self.alloc_register()?;
            self.compile_expr(ptr_expr, ptr_reg)?;
            self.emit_a(OpCode::LoadMemI, dest, ptr_reg, *offset as u8, span);
            self.free_register(ptr_reg);
            return Ok(());
        }

        let ptr_reg = self.alloc_register()?;
        let offset_reg = self.alloc_register()?;
        self.compile_expr(ptr_expr, ptr_reg)?;
        self.compile_expr(offset_expr, offset_reg)?;
        self.emit_a(OpCode::LoadMem, dest, ptr_reg, offset_reg, span);
        self.free_register(offset_reg);
        self.free_register(ptr_reg);
        Ok(())
    }

    fn compile_builtin_store(
        &mut self,
        ptr_expr: &Expr,
        offset_expr: &Expr,
        value_expr: &Expr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        if let ExprKind::Int(offset) = &offset_expr.kind
            && *offset >= 0
            && *offset <= 255
        {
            let ptr_reg = self.alloc_register()?;
            let val_reg = self.alloc_register()?;
            self.compile_expr(ptr_expr, ptr_reg)?;
            self.compile_expr(value_expr, val_reg)?;
            self.emit_a(OpCode::StoreMemI, ptr_reg, *offset as u8, val_reg, span);
            self.free_register(val_reg);
            self.free_register(ptr_reg);
            self.emit_a(OpCode::LoadNull, dest, 0, 0, span);
            return Ok(());
        }

        let ptr_reg = self.alloc_register()?;
        let offset_reg = self.alloc_register()?;
        let val_reg = self.alloc_register()?;
        self.compile_expr(ptr_expr, ptr_reg)?;
        self.compile_expr(offset_expr, offset_reg)?;
        self.compile_expr(value_expr, val_reg)?;
        self.emit_a(OpCode::StoreMem, ptr_reg, offset_reg, val_reg, span);
        self.free_register(val_reg);
        self.free_register(offset_reg);
        self.free_register(ptr_reg);
        self.emit_a(OpCode::LoadNull, dest, 0, 0, span);
        Ok(())
    }
}

use super::super::Compiler;
use aelys_bytecode::OpCode;
use aelys_common::Result;
use aelys_sema::{InferType, ResolvedType, TypedExpr};
use aelys_syntax::Span;

impl Compiler {
    pub(super) fn compile_typed_array_sized(
        &mut self,
        element_type: &Option<ResolvedType>,
        size: &TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        // Compile size expression
        let size_reg = self.alloc_register()?;
        self.compile_typed_expr(size, size_reg)?;

        // Select opcode based on element type
        let opcode = match element_type {
            Some(t) if t.is_integer() => OpCode::ArrayNewI,
            Some(t) if t.is_float() => OpCode::ArrayNewF,
            Some(ResolvedType::Bool) => OpCode::ArrayNewB,
            _ => OpCode::ArrayNewP,
        };

        self.emit_a(opcode, dest, size_reg, 0, span);
        self.free_register(size_reg);
        Ok(())
    }

    pub(super) fn compile_typed_array_literal(
        &mut self,
        expr_ty: &InferType,
        elements: &[TypedExpr],
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let count = elements.len();

        if count == 0 {
            let opcode = if let InferType::Array(inner) = expr_ty {
                Self::select_typed_opcode(
                    inner,
                    OpCode::ArrayNewI,
                    OpCode::ArrayNewF,
                    OpCode::ArrayNewB,
                    OpCode::ArrayNewP,
                )
            } else {
                OpCode::ArrayNewP
            };
            self.emit_a(opcode, dest, 0, 0, span);
            return Ok(());
        }

        let start_reg = self.alloc_consecutive_registers_for_call(count as u8, span)?;

        for i in 0..count {
            let reg = start_reg + i as u8;
            self.register_pool[reg as usize] = true;
            if reg >= self.next_register {
                self.next_register = reg + 1;
            }
        }

        for (i, elem) in elements.iter().enumerate() {
            let elem_reg = start_reg + i as u8;
            self.compile_typed_expr(elem, elem_reg)?;
        }

        self.emit_a(OpCode::ArrayLit, dest, start_reg, count as u8, span);

        for i in (0..count).rev() {
            let reg = start_reg + i as u8;
            self.register_pool[reg as usize] = false;
        }

        Ok(())
    }

    pub(super) fn compile_typed_vec_literal(
        &mut self,
        expr_ty: &InferType,
        elements: &[TypedExpr],
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let count = elements.len();

        if count == 0 {
            let opcode = if let InferType::Vec(inner) = expr_ty {
                Self::select_typed_opcode(
                    inner,
                    OpCode::VecNewI,
                    OpCode::VecNewF,
                    OpCode::VecNewB,
                    OpCode::VecNewP,
                )
            } else {
                OpCode::VecNewP
            };
            self.emit_a(opcode, dest, 0, 0, span);
            return Ok(());
        }

        let start_reg = self.alloc_consecutive_registers_for_call(count as u8, span)?;

        for i in 0..count {
            let reg = start_reg + i as u8;
            self.register_pool[reg as usize] = true;
            if reg >= self.next_register {
                self.next_register = reg + 1;
            }
        }

        for (i, elem) in elements.iter().enumerate() {
            let elem_reg = start_reg + i as u8;
            self.compile_typed_expr(elem, elem_reg)?;
        }

        self.emit_a(OpCode::VecLit, dest, start_reg, count as u8, span);

        for i in (0..count).rev() {
            let reg = start_reg + i as u8;
            self.register_pool[reg as usize] = false;
        }

        Ok(())
    }

    pub(super) fn compile_typed_index_access(
        &mut self,
        object: &TypedExpr,
        index: &TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let obj_reg = self.alloc_register()?;
        self.compile_typed_expr(object, obj_reg)?;

        let idx_reg = self.alloc_register()?;
        self.compile_typed_expr(index, idx_reg)?;

        let opcode = match &object.ty {
            InferType::Vec(inner) => Self::select_typed_opcode(
                inner,
                OpCode::VecLoadI,
                OpCode::VecLoadF,
                OpCode::VecLoadB,
                OpCode::VecLoadP,
            ),
            InferType::Array(inner) => Self::select_typed_opcode(
                inner,
                OpCode::ArrayLoadI,
                OpCode::ArrayLoadF,
                OpCode::ArrayLoadB,
                OpCode::ArrayLoadP,
            ),
            InferType::String => OpCode::StringLoadChar,
            _ => OpCode::VecLoadP,
        };

        self.emit_a(opcode, dest, obj_reg, idx_reg, span);

        self.free_register(idx_reg);
        self.free_register(obj_reg);

        Ok(())
    }

    pub(super) fn compile_typed_index_assign(
        &mut self,
        object: &TypedExpr,
        index: &TypedExpr,
        value: &TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let obj_reg = self.alloc_register()?;
        self.compile_typed_expr(object, obj_reg)?;

        let idx_reg = self.alloc_register()?;
        self.compile_typed_expr(index, idx_reg)?;

        let val_reg = self.alloc_register()?;
        self.compile_typed_expr(value, val_reg)?;

        let opcode = match &object.ty {
            InferType::Vec(inner) => Self::select_typed_opcode(
                inner,
                OpCode::VecStoreI,
                OpCode::VecStoreF,
                OpCode::VecStoreB,
                OpCode::VecStoreP,
            ),
            InferType::Array(inner) => Self::select_typed_opcode(
                inner,
                OpCode::ArrayStoreI,
                OpCode::ArrayStoreF,
                OpCode::ArrayStoreB,
                OpCode::ArrayStoreP,
            ),
            _ => OpCode::VecStoreP,
        };

        self.emit_a(opcode, obj_reg, idx_reg, val_reg, span);

        if dest != val_reg {
            self.emit_a(OpCode::Move, dest, val_reg, 0, span);
        }

        self.free_register(val_reg);
        self.free_register(idx_reg);
        self.free_register(obj_reg);

        Ok(())
    }

    pub(super) fn compile_typed_slice(
        &mut self,
        _object: &TypedExpr,
        _range: &TypedExpr,
        _dest: u8,
        _span: Span,
    ) -> Result<()> {
        todo!("slice")
    }

    pub(super) fn compile_typed_range(
        &mut self,
        _start: &Option<Box<TypedExpr>>,
        _end: &Option<Box<TypedExpr>>,
        _inclusive: bool,
        _dest: u8,
        _span: Span,
    ) -> Result<()> {
        todo!("range")
    }

    fn select_typed_opcode(
        inner: &InferType,
        i: OpCode,
        f: OpCode,
        b: OpCode,
        p: OpCode,
    ) -> OpCode {
        match inner {
            t if t.is_integer() => i,
            t if t.is_float() => f,
            InferType::Bool => b,
            _ => p,
        }
    }

    pub(super) fn compile_array_len(
        &mut self,
        object: &TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let obj_reg = self.alloc_register()?;
        self.compile_typed_expr(object, obj_reg)?;
        self.emit_a(OpCode::ArrayLen, dest, obj_reg, 0, span);
        self.free_register(obj_reg);
        Ok(())
    }

    pub(super) fn compile_vec_len(
        &mut self,
        object: &TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let obj_reg = self.alloc_register()?;
        self.compile_typed_expr(object, obj_reg)?;
        self.emit_a(OpCode::VecLen, dest, obj_reg, 0, span);
        self.free_register(obj_reg);
        Ok(())
    }

    pub(super) fn compile_vec_push(
        &mut self,
        object: &TypedExpr,
        inner: &InferType,
        value: &TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let obj_reg = self.alloc_register()?;
        self.compile_typed_expr(object, obj_reg)?;

        let val_reg = self.alloc_register()?;
        self.compile_typed_expr(value, val_reg)?;

        let opcode = Self::select_typed_opcode(
            inner,
            OpCode::VecPushI,
            OpCode::VecPushF,
            OpCode::VecPushB,
            OpCode::VecPushP,
        );

        self.emit_a(opcode, obj_reg, val_reg, 0, span);
        self.emit_a(OpCode::LoadNull, dest, 0, 0, span);

        self.free_register(val_reg);
        self.free_register(obj_reg);
        Ok(())
    }

    pub(super) fn compile_vec_pop(
        &mut self,
        object: &TypedExpr,
        inner: &InferType,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let obj_reg = self.alloc_register()?;
        self.compile_typed_expr(object, obj_reg)?;

        let opcode = Self::select_typed_opcode(
            inner,
            OpCode::VecPopI,
            OpCode::VecPopF,
            OpCode::VecPopB,
            OpCode::VecPopP,
        );

        self.emit_a(opcode, dest, obj_reg, 0, span);
        self.free_register(obj_reg);
        Ok(())
    }

    pub(super) fn compile_vec_capacity(
        &mut self,
        object: &TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let obj_reg = self.alloc_register()?;
        self.compile_typed_expr(object, obj_reg)?;
        self.emit_a(OpCode::VecCap, dest, obj_reg, 0, span);
        self.free_register(obj_reg);
        Ok(())
    }

    pub(super) fn compile_vec_reserve(
        &mut self,
        object: &TypedExpr,
        capacity: &TypedExpr,
        dest: u8,
        span: Span,
    ) -> Result<()> {
        let obj_reg = self.alloc_register()?;
        self.compile_typed_expr(object, obj_reg)?;

        let cap_reg = self.alloc_register()?;
        self.compile_typed_expr(capacity, cap_reg)?;

        self.emit_a(OpCode::VecReserve, obj_reg, cap_reg, 0, span);
        self.emit_a(OpCode::LoadNull, dest, 0, 0, span);

        self.free_register(cap_reg);
        self.free_register(obj_reg);
        Ok(())
    }
}

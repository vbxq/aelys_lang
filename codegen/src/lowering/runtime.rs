use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::types::aelys_string_type;
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::module::Linkage;
use inkwell::values::{BasicValueEnum, FunctionValue, IntValue, PointerValue, StructValue};

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn ensure_alloc_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_alloc") {
            return function;
        }

        let fn_ty = self
            .context
            .ptr_type(AddressSpace::default())
            .fn_type(&[self.context.i64_type().into()], false);
        self.module.add_function("__aelys_alloc", fn_ty, None)
    }

    pub(crate) fn ensure_free_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_free") {
            return function;
        }

        let fn_ty = self.context.void_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("__aelys_free", fn_ty, None)
    }

    pub(crate) fn ensure_write_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_write") {
            return function;
        }

        let fn_ty = self.context.void_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i64_type().into(),
            ],
            false,
        );
        self.module.add_function("__aelys_write", fn_ty, None)
    }

    pub(crate) fn ensure_panic_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_panic") {
            return function;
        }

        let fn_ty = self.context.void_type().fn_type(
            &[
                self.context.ptr_type(AddressSpace::default()).into(),
                self.context.i64_type().into(),
            ],
            false,
        );
        self.module.add_function("__aelys_panic", fn_ty, None)
    }

    /// `__aelys_str_char_at(str, i64) -> str`
    /// UTF-8 character indexing: returns the i-th Unicode codepoint as a
    /// single-character string. Panics internally on OOB.
    pub(crate) fn ensure_str_char_at_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_str_char_at") {
            return function;
        }

        let string_ty = aelys_string_type(self.context);
        let fn_ty = string_ty.fn_type(&[string_ty.into(), self.context.i64_type().into()], false);
        self.module.add_function("__aelys_str_char_at", fn_ty, None)
    }

    pub(crate) fn global_string_ptr_len(
        &mut self,
        text: &str,
    ) -> Result<(PointerValue<'static>, u64), CodegenError> {
        let i8_ty = self.context.i8_type();
        let text_len = u64::try_from(text.len()).map_err(|_| {
            CodegenError::UnsupportedInstruction("string literal too large".to_string())
        })?;
        let array_len = u32::try_from(text.len())
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| {
                CodegenError::UnsupportedInstruction("string literal too large".to_string())
            })?;

        let global_ptr = if let Some(existing) = self.string_globals.get(text).copied() {
            existing
        } else {
            let name = format!("str_{}_{}", self.air_function.id.0, self.string_id);
            self.string_id = self.string_id.saturating_add(1);

            let mut bytes = Vec::with_capacity(text.len() + 1);
            for byte in text.as_bytes() {
                bytes.push(i8_ty.const_int(u64::from(*byte), false));
            }
            bytes.push(i8_ty.const_zero());

            let global = self
                .module
                .add_global(i8_ty.array_type(array_len), None, &name);
            global.set_linkage(Linkage::Private);
            global.set_constant(true);
            global.set_initializer(&i8_ty.const_array(&bytes));
            let ptr = global.as_pointer_value();
            self.string_globals.insert(text.to_string(), ptr);
            ptr
        };

        let array_ty = i8_ty.array_type(array_len);

        let zero = self.context.i64_type().const_zero();
        let ptr = unsafe {
            self.builder
                .build_in_bounds_gep(array_ty, global_ptr, &[zero, zero], "str_ptr")
        }
        .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        Ok((ptr, text_len))
    }

    pub(crate) fn global_string_value(
        &mut self,
        text: &str,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let (ptr, len) = self.global_string_ptr_len(text)?;
        let string_ty = aelys_string_type(self.context);
        let value = self
            .builder
            .build_insert_value(string_ty.get_undef(), ptr, 0, "str_init_ptr")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?
            .into_struct_value();
        let value = self
            .builder
            .build_insert_value(
                value,
                self.context.i64_type().const_int(len, false),
                1,
                "str_init_len",
            )
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?
            .into_struct_value();
        Ok(value.into())
    }

    pub(crate) fn string_parts_from_value(
        &self,
        value: StructValue<'static>,
    ) -> Result<(PointerValue<'static>, IntValue<'static>), CodegenError> {
        let ptr = self
            .builder
            .build_extract_value(value, 0, "str_ptr")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?
            .into_pointer_value();
        let len = self
            .builder
            .build_extract_value(value, 1, "str_len")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?
            .into_int_value();
        Ok((ptr, len))
    }

    /// Emit a bounds check: if `index >= length` (unsigned), branch to a panic
    /// block; otherwise continue in a new `idx_ok` block.
    pub(crate) fn emit_bounds_check(
        &mut self,
        index: IntValue<'static>,
        length: IntValue<'static>,
    ) -> Result<(), CodegenError> {
        let oob = self
            .builder
            .build_int_compare(IntPredicate::UGE, index, length, "idx_oob_cmp")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        let current_fn = self.function;
        let oob_block = self.context.append_basic_block(current_fn, "idx_oob");
        let ok_block = self.context.append_basic_block(current_fn, "idx_ok");

        self.builder
            .build_conditional_branch(oob, oob_block, ok_block)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        // OOB block: call __aelys_panic + unreachable
        self.builder.position_at_end(oob_block);
        let panic_fn = self.ensure_panic_function();
        let (msg_ptr, msg_len) = self.global_string_ptr_len("index out of bounds")?;
        let msg_len_val = self.context.i64_type().const_int(msg_len, false);
        self.builder
            .build_call(panic_fn, &[msg_ptr.into(), msg_len_val.into()], "")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        self.builder
            .build_unreachable()
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        // Continue building in the ok block
        self.builder.position_at_end(ok_block);
        Ok(())
    }
}

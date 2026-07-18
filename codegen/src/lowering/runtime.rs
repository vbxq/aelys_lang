use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::types::aelys_string_type;
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::values::{FunctionValue, IntValue};

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

    pub(crate) fn ensure_rc_retain_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_rc_retain") {
            return function;
        }
        let fn_ty = self.context.void_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("__aelys_rc_retain", fn_ty, None)
    }

    pub(crate) fn ensure_rc_release_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_rc_release") {
            return function;
        }
        let fn_ty = self.context.void_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("__aelys_rc_release", fn_ty, None)
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
        // Trivial optimization, LLVM declaration doesn't add `noreturn` attribute, so it basically
        // can't optimize based on the fact that panic never returns
        // yeah I be fixing up the most useless stuff possible
        let function = self.module.add_function("__aelys_panic", fn_ty, None);
        let noreturn_id = inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn");
        let noreturn_attr = self.context.create_enum_attribute(noreturn_id, 0);
        function.add_attribute(inkwell::attributes::AttributeLoc::Function, noreturn_attr);
        function
    }

    // sret-returning runtime functions (return %__aelys_string)

    // on windows x64 MSVC, struct returns use sret (first param = ptr to result slot)
    // because LLVM and MSVC disagree on 16-byte struct lowering (ce7dd07)

    /// Helper: declare a runtime function that returns %__aelys_string.
    /// Handles the windows sret ABI automatically.
    fn declare_string_returning_fn(
        &self,
        name: &str,
        params: &[inkwell::types::BasicMetadataTypeEnum<'static>],
    ) -> FunctionValue<'static> {
        if let Some(f) = self.module.get_function(name) {
            return f;
        }
        let string_ty = aelys_string_type(self.context);
        let use_sret = self.target_is_windows();
        let fn_ty = if use_sret {
            let mut all_params = vec![self.context.ptr_type(AddressSpace::default()).into()];
            all_params.extend_from_slice(params);
            self.context.void_type().fn_type(&all_params, false)
        } else {
            string_ty.fn_type(params, false)
        };
        let function = self.module.add_function(name, fn_ty, None);
        if use_sret {
            use inkwell::attributes::AttributeLoc;
            let sret_attr = self.context.create_type_attribute(
                inkwell::attributes::Attribute::get_named_enum_kind_id("sret"),
                string_ty.into(),
            );
            function.add_attribute(AttributeLoc::Param(0), sret_attr);
        }
        function
    }

    pub(crate) fn ensure_str_char_at_function(&self) -> FunctionValue<'static> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        let i64_ty = self.context.i64_type().into();
        self.declare_string_returning_fn("__aelys_str_char_at", &[ptr_ty, i64_ty, i64_ty])
    }

    pub(crate) fn ensure_to_string_i64_function(&self) -> FunctionValue<'static> {
        self.declare_string_returning_fn("__aelys_to_string_i64", &[self.context.i64_type().into()])
    }

    pub(crate) fn ensure_to_string_f64_function(&self) -> FunctionValue<'static> {
        self.declare_string_returning_fn("__aelys_to_string_f64", &[self.context.f64_type().into()])
    }

    pub(crate) fn ensure_to_string_bool_function(&self) -> FunctionValue<'static> {
        // bool is passed as i64 (0 or 1) to the C runtime
        self.declare_string_returning_fn(
            "__aelys_to_string_bool",
            &[self.context.i64_type().into()],
        )
    }

    /// `__aelys_str_eq(ptr, i64, ptr, i64) -> i64`
    /// flat ABI: (a_ptr, a_len, b_ptr, b_len) to avoid struct passing on windows x64
    pub(crate) fn ensure_str_eq_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_str_eq") {
            return function;
        }

        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        let i64_ty = self.context.i64_type().into();
        let fn_ty = self
            .context
            .i64_type()
            .fn_type(&[ptr_ty, i64_ty, ptr_ty, i64_ty], false);
        self.module.add_function("__aelys_str_eq", fn_ty, None)
    }

    pub(crate) fn ensure_str_concat_function(&self) -> FunctionValue<'static> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        let i64_ty = self.context.i64_type().into();
        self.declare_string_returning_fn("__aelys_str_concat", &[ptr_ty, i64_ty, ptr_ty, i64_ty])
    }

    /// Emit a division-by-zero check: if `divisor == 0`, branch to a panic
    /// block; otherwise continue in a new `div_ok` block.
    pub(crate) fn emit_div_zero_check(
        &mut self,
        divisor: IntValue<'static>,
    ) -> Result<(), CodegenError> {
        let is_zero = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                divisor,
                divisor.get_type().const_zero(),
                "div_zero_cmp",
            )
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        let current_fn = self.function;
        let trap_block = self.context.append_basic_block(current_fn, "div_zero");
        let ok_block = self.context.append_basic_block(current_fn, "div_ok");

        self.builder
            .build_conditional_branch(is_zero, trap_block, ok_block)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        self.builder.position_at_end(trap_block);
        let panic_fn = self.ensure_panic_function();
        let (msg_ptr, msg_len) = self.global_string_ptr_len("division by zero")?;
        let msg_len_val = self.context.i64_type().const_int(msg_len, false);
        self.builder
            .build_call(panic_fn, &[msg_ptr.into(), msg_len_val.into()], "")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        self.builder
            .build_unreachable()
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        self.builder.position_at_end(ok_block);
        Ok(())
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

        self.builder.position_at_end(ok_block);
        Ok(())
    }
}

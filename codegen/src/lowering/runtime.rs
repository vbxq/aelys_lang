use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::lowering::stmts::RC_HEADER_SIZE;

// must stay equal to aelys_rc_dead in core/src/aelys_rc.h
const RC_DEAD: u32 = 0xAE11_DEAD;
use crate::types::aelys_string_type;
use aelys_air::{AirType, LocalId};
use inkwell::AddressSpace;
use inkwell::IntPredicate;
use inkwell::values::{FunctionValue, IntValue, PointerValue};

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

    fn last_freed_global(&self) -> PointerValue<'static> {
        match self.module.get_global("__aelys_last_freed") {
            Some(global) => global.as_pointer_value(),
            None => self
                .module
                .add_global(self.context.i64_type(), None, "__aelys_last_freed")
                .as_pointer_value(),
        }
    }

    // the tombstone is read before the header, which a freed cell may no longer hold
    pub(crate) fn emit_inline_str_count(
        &mut self,
        retain: bool,
        slot: PointerValue<'static>,
    ) -> Result<(), CodegenError> {
        let slow_fn = if retain {
            self.ensure_rc_retain_checked_function()
        } else {
            self.ensure_rc_release_function()
        };
        let ptr_ty = self.context.ptr_type(AddressSpace::default());
        let i8_ty = self.context.i8_type();
        let i32_ty = self.context.i32_type();
        let i64_ty = self.context.i64_type();
        let err = |e: inkwell::builder::BuilderError| CodegenError::LlvmError(e.to_string());

        let data_ptr = self
            .builder
            .build_load(ptr_ty, slot, "str_rc_data")
            .map_err(err)?
            .into_pointer_value();
        let current_fn = self.function;
        let tomb_block = self.context.append_basic_block(current_fn, "str_rc_tomb");
        let count_block = self.context.append_basic_block(current_fn, "str_rc_count");
        let test_block = self.context.append_basic_block(current_fn, "str_rc_test");
        let fast_block = self.context.append_basic_block(current_fn, "str_rc_fast");
        let slow_block = self.context.append_basic_block(current_fn, "str_rc_slow");
        let cont_block = self.context.append_basic_block(current_fn, "str_rc_cont");

        let is_nonnull = self
            .builder
            .build_is_not_null(data_ptr, "str_rc_nn")
            .map_err(err)?;
        self.builder
            .build_conditional_branch(is_nonnull, tomb_block, cont_block)
            .map_err(err)?;

        self.builder.position_at_end(tomb_block);
        let neg_header = i64_ty.const_int((RC_HEADER_SIZE as i64).wrapping_neg() as u64, true);
        let header_ptr = unsafe {
            self.builder
                .build_in_bounds_gep(i8_ty, data_ptr, &[neg_header], "str_rc_hdr")
                .map_err(err)?
        };
        let last_freed = self
            .builder
            .build_load(i64_ty, self.last_freed_global(), "str_rc_last_freed")
            .map_err(err)?
            .into_int_value();
        let header_addr = self
            .builder
            .build_ptr_to_int(header_ptr, i64_ty, "str_rc_hdr_addr")
            .map_err(err)?;
        let is_tomb = self
            .builder
            .build_int_compare(IntPredicate::EQ, header_addr, last_freed, "str_rc_is_tomb")
            .map_err(err)?;
        self.builder
            .build_conditional_branch(is_tomb, slow_block, count_block)
            .map_err(err)?;

        self.builder.position_at_end(count_block);
        let count = self
            .builder
            .build_load(i32_ty, header_ptr, "str_rc_count")
            .map_err(err)?
            .into_int_value();
        let is_immortal = self
            .builder
            .build_int_compare(
                IntPredicate::EQ,
                count,
                i32_ty.const_int(u32::MAX as u64, false),
                "str_rc_immortal",
            )
            .map_err(err)?;
        self.builder
            .build_conditional_branch(is_immortal, cont_block, test_block)
            .map_err(err)?;

        self.builder.position_at_end(test_block);
        let not_dead = self
            .builder
            .build_int_compare(
                IntPredicate::NE,
                count,
                i32_ty.const_int(RC_DEAD as u64, false),
                "str_rc_not_dead",
            )
            .map_err(err)?;
        let fast = if retain {
            // one below dead saturates in the runtime, or the next count would read as freed
            let from_edge = self
                .builder
                .build_int_sub(
                    count,
                    i32_ty.const_int((RC_DEAD - 1) as u64, false),
                    "str_rc_from_edge",
                )
                .map_err(err)?;
            self.builder
                .build_int_compare(
                    IntPredicate::UGT,
                    from_edge,
                    i32_ty.const_int(1, false),
                    "str_rc_off_edge",
                )
                .map_err(err)?
        } else {
            let shared = self
                .builder
                .build_int_compare(
                    IntPredicate::UGT,
                    count,
                    i32_ty.const_int(1, false),
                    "str_rc_shared",
                )
                .map_err(err)?;
            self.builder
                .build_and(not_dead, shared, "str_rc_fast_ok")
                .map_err(err)?
        };
        self.builder
            .build_conditional_branch(fast, fast_block, slow_block)
            .map_err(err)?;

        self.builder.position_at_end(fast_block);
        let one = i32_ty.const_int(1, false);
        let next = if retain {
            self.builder.build_int_add(count, one, "str_rc_inc")
        } else {
            self.builder.build_int_sub(count, one, "str_rc_dec")
        }
        .map_err(err)?;
        self.builder.build_store(header_ptr, next).map_err(err)?;
        self.builder
            .build_unconditional_branch(cont_block)
            .map_err(err)?;

        self.builder.position_at_end(slow_block);
        self.builder
            .build_call(slow_fn, &[data_ptr.into()], "")
            .map_err(err)?;
        self.builder
            .build_unconditional_branch(cont_block)
            .map_err(err)?;

        self.builder.position_at_end(cont_block);
        Ok(())
    }

    fn ensure_rc_retain_checked_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_rc_retain_checked") {
            return function;
        }
        let fn_ty = self.context.void_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module
            .add_function("__aelys_rc_retain_checked", fn_ty, None)
    }

    pub(crate) fn ensure_vec_detach_function(&self, strings: bool) -> FunctionValue<'static> {
        let name = if strings {
            "__aelys_vec_detach_str"
        } else {
            "__aelys_vec_detach"
        };
        if let Some(function) = self.module.get_function(name) {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        let i64_ty = self.context.i64_type().into();
        let fn_ty = self
            .context
            .void_type()
            .fn_type(&[ptr_ty, i64_ty, i64_ty], false);
        self.module.add_function(name, fn_ty, None)
    }

    pub(crate) fn vec_elem_glue(
        &self,
        elem: &AirType,
        retain: bool,
    ) -> Option<FunctionValue<'static>> {
        if *elem == AirType::Str {
            return None;
        }
        let carriers =
            aelys_air::counts::Carriers::merged(&self.program.structs, &self.program.enums);
        if !carriers.carries_string(elem) {
            return None;
        }
        self.module
            .get_function(&aelys_air::counts::glue_name(elem, retain))
    }

    pub(crate) fn ensure_vec_detach_glue_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_vec_detach_glue") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        let i64_ty = self.context.i64_type().into();
        let fn_ty = self
            .context
            .void_type()
            .fn_type(&[ptr_ty, i64_ty, i64_ty, ptr_ty], false);
        self.module
            .add_function("__aelys_vec_detach_glue", fn_ty, None)
    }

    pub(crate) fn ensure_vec_release_glue_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_vec_release_glue") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        let i64_ty = self.context.i64_type().into();
        let fn_ty = self
            .context
            .void_type()
            .fn_type(&[ptr_ty, i64_ty, ptr_ty], false);
        self.module
            .add_function("__aelys_vec_release_glue", fn_ty, None)
    }

    pub(crate) fn ensure_vec_release_str_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_vec_release_str") {
            return function;
        }
        let fn_ty = self.context.void_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module
            .add_function("__aelys_vec_release_str", fn_ty, None)
    }

    /// itself in an out-of-line cold block, so an unshared write pays no call and never allocates.
    pub(crate) fn emit_vec_detach(
        &mut self,
        local: LocalId,
        inner: &AirType,
        through_ptr: bool,
    ) -> Result<(), CodegenError> {
        let glue = self.vec_elem_glue(inner, true);
        let detach_fn = match glue {
            Some(_) => self.ensure_vec_detach_glue_function(),
            None => self.ensure_vec_detach_function(*inner == AirType::Str),
        };
        let elem_size = self.air_type_size(inner)? as u64;
        let v_alloca = if through_ptr {
            let p = self.load_local(local)?.into_pointer_value();
            self.emit_null_check(p)?;
            p
        } else {
            self.lookup_local_ptr(local)?
        };
        let ptr_ty = self.context.ptr_type(AddressSpace::default());

        let data_ptr = self
            .builder
            .build_load(ptr_ty, v_alloca, "cow_dataptr")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?
            .into_pointer_value();

        let current_fn = self.function;
        let chk_block = self.context.append_basic_block(current_fn, "cow_chk");
        let slow_block = self.context.append_basic_block(current_fn, "cow_slow");
        let cont_block = self.context.append_basic_block(current_fn, "cow_cont");

        let is_nonnull = self
            .builder
            .build_is_not_null(data_ptr, "cow_nn")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        self.builder
            .build_conditional_branch(is_nonnull, chk_block, cont_block)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        self.builder.position_at_end(chk_block);
        let i8_ty = self.context.i8_type();
        let neg_header = self
            .context
            .i64_type()
            .const_int((RC_HEADER_SIZE as i64).wrapping_neg() as u64, true);
        let header_ptr = unsafe {
            self.builder
                .build_in_bounds_gep(i8_ty, data_ptr, &[neg_header], "cow_hdr")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?
        };
        let i32_ty = self.context.i32_type();
        let refcount = self
            .builder
            .build_load(i32_ty, header_ptr, "cow_rc")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?
            .into_int_value();
        let shared = self
            .builder
            .build_int_compare(
                IntPredicate::UGT,
                refcount,
                i32_ty.const_int(1, false),
                "cow_shared",
            )
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        self.builder
            .build_conditional_branch(shared, slow_block, cont_block)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        self.builder.position_at_end(slow_block);
        let elem_size_val = self.context.i64_type().const_int(elem_size, false);
        let zero = self.context.i64_type().const_zero();
        let mut detach_args: Vec<inkwell::values::BasicMetadataValueEnum<'static>> =
            vec![v_alloca.into(), elem_size_val.into(), zero.into()];
        if let Some(dup) = glue {
            detach_args.push(dup.as_global_value().as_pointer_value().into());
        }
        self.builder
            .build_call(detach_fn, &detach_args, "")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        self.builder
            .build_unconditional_branch(cont_block)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        self.builder.position_at_end(cont_block);
        Ok(())
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
        // can't optimize based on the fact that panic never returns
        let function = self.module.add_function("__aelys_panic", fn_ty, None);
        let noreturn_id = inkwell::attributes::Attribute::get_named_enum_kind_id("noreturn");
        let noreturn_attr = self.context.create_enum_attribute(noreturn_id, 0);
        function.add_attribute(inkwell::attributes::AttributeLoc::Function, noreturn_attr);
        function
    }



    /// handles the windows sret abi automatically.
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

    pub(crate) fn ensure_str_char_at_scalar_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_str_char_at_scalar") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        let i64_ty = self.context.i64_type().into();
        let fn_ty = self
            .context
            .i32_type()
            .fn_type(&[ptr_ty, i64_ty, i64_ty], false);
        self.module
            .add_function("__aelys_str_char_at_scalar", fn_ty, None)
    }

    pub(crate) fn ensure_str_decode_at_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_str_decode_at") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        let i64_ty = self.context.i64_type().into();
        let fn_ty = self
            .context
            .i64_type()
            .fn_type(&[ptr_ty, i64_ty, i64_ty], false);
        self.module.add_function("__aelys_str_decode_at", fn_ty, None)
    }

    pub(crate) fn ensure_char_from_i64_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_char_from_i64") {
            return function;
        }
        let fn_ty = self
            .context
            .i32_type()
            .fn_type(&[self.context.i64_type().into()], false);
        self.module.add_function("__aelys_char_from_i64", fn_ty, None)
    }

    pub(crate) fn ensure_char_is_scalar_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_char_is_scalar") {
            return function;
        }
        let fn_ty = self
            .context
            .i64_type()
            .fn_type(&[self.context.i64_type().into()], false);
        self.module
            .add_function("__aelys_char_is_scalar", fn_ty, None)
    }

    pub(crate) fn ensure_str_from_char_function(&self) -> FunctionValue<'static> {
        self.declare_string_returning_fn(
            "__aelys_str_from_char",
            &[self.context.i32_type().into()],
        )
    }

    pub(crate) fn ensure_to_string_char_function(&self) -> FunctionValue<'static> {
        self.declare_string_returning_fn(
            "__aelys_to_string_char",
            &[self.context.i32_type().into()],
        )
    }

    pub(crate) fn ensure_to_string_char_into_function(&self) -> FunctionValue<'static> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        self.declare_string_returning_fn(
            "__aelys_to_string_char_into",
            &[ptr_ty, self.context.i32_type().into()],
        )
    }

    pub(crate) fn ensure_str_char_count_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_str_char_count") {
            return function;
        }
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        let i64_ty = self.context.i64_type().into();
        let fn_ty = self.context.i64_type().fn_type(&[ptr_ty, i64_ty], false);
        self.module
            .add_function("__aelys_str_char_count", fn_ty, None)
    }

    pub(crate) fn ensure_to_string_i64_function(&self) -> FunctionValue<'static> {
        self.declare_string_returning_fn("__aelys_to_string_i64", &[self.context.i64_type().into()])
    }

    pub(crate) fn ensure_to_string_f64_function(&self) -> FunctionValue<'static> {
        self.declare_string_returning_fn("__aelys_to_string_f64", &[self.context.f64_type().into()])
    }

    pub(crate) fn ensure_to_string_i64_into_function(&self) -> FunctionValue<'static> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        self.declare_string_returning_fn(
            "__aelys_to_string_i64_into",
            &[ptr_ty, self.context.i64_type().into()],
        )
    }

    pub(crate) fn ensure_to_string_f64_into_function(&self) -> FunctionValue<'static> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        self.declare_string_returning_fn(
            "__aelys_to_string_f64_into",
            &[ptr_ty, self.context.f64_type().into()],
        )
    }

    pub(crate) fn ensure_to_string_bool_function(&self) -> FunctionValue<'static> {
        self.declare_string_returning_fn(
            "__aelys_to_string_bool",
            &[self.context.i64_type().into()],
        )
    }

    /// flat abi: (a_ptr, a_len, b_ptr, b_len) to avoid struct passing on windows x64
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

    pub(crate) fn ensure_str_substring_bytes_function(&self) -> FunctionValue<'static> {
        let ptr_ty = self.context.ptr_type(AddressSpace::default()).into();
        let i64_ty = self.context.i64_type().into();
        self.declare_string_returning_fn(
            "__aelys_str_substring_bytes",
            &[ptr_ty, i64_ty, i64_ty, i64_ty],
        )
    }

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

    /// emit a bounds check: if `index >= length` (unsigned), branch to a panic
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

    pub(crate) fn emit_slice_len_check(
        &mut self,
        len: IntValue<'static>,
        base_len: IntValue<'static>,
    ) -> Result<(), CodegenError> {
        let oob = self
            .builder
            .build_int_compare(IntPredicate::UGT, len, base_len, "slice_oob_cmp")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        let current_fn = self.function;
        let oob_block = self.context.append_basic_block(current_fn, "slice_oob");
        let ok_block = self.context.append_basic_block(current_fn, "slice_ok");

        self.builder
            .build_conditional_branch(oob, oob_block, ok_block)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        self.builder.position_at_end(oob_block);
        let panic_fn = self.ensure_panic_function();
        let (msg_ptr, msg_len) = self.global_string_ptr_len("slice range out of bounds")?;
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

    pub(crate) fn emit_div_overflow_check(
        &mut self,
        dividend: IntValue<'static>,
        divisor: IntValue<'static>,
    ) -> Result<(), CodegenError> {
        let int_ty = dividend.get_type();
        let min_bits = 1u64 << (int_ty.get_bit_width() - 1);
        let int_min = int_ty.const_int(min_bits, false);
        let neg_one = int_ty.const_all_ones();

        let is_min = self
            .builder
            .build_int_compare(IntPredicate::EQ, dividend, int_min, "div_ovf_min")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        let is_neg_one = self
            .builder
            .build_int_compare(IntPredicate::EQ, divisor, neg_one, "div_ovf_neg1")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        let overflow = self
            .builder
            .build_and(is_min, is_neg_one, "div_ovf")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        let current_fn = self.function;
        let trap_block = self.context.append_basic_block(current_fn, "div_ovf_trap");
        let ok_block = self.context.append_basic_block(current_fn, "div_ovf_ok");

        self.builder
            .build_conditional_branch(overflow, trap_block, ok_block)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        self.builder.position_at_end(trap_block);
        let panic_fn = self.ensure_panic_function();
        let (msg_ptr, msg_len) = self.global_string_ptr_len("division overflow")?;
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

    pub(crate) fn emit_null_check(
        &mut self,
        ptr: PointerValue<'static>,
    ) -> Result<(), CodegenError> {
        let is_null = self
            .builder
            .build_is_null(ptr, "deref_null_cmp")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        let current_fn = self.function;
        let trap_block = self.context.append_basic_block(current_fn, "deref_null");
        let ok_block = self.context.append_basic_block(current_fn, "deref_ok");

        self.builder
            .build_conditional_branch(is_null, trap_block, ok_block)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        self.builder.position_at_end(trap_block);
        let panic_fn = self.ensure_panic_function();
        let (msg_ptr, msg_len) = self.global_string_ptr_len("null pointer dereference")?;
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

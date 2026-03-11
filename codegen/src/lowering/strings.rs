use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::types::aelys_string_type;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::module::Linkage;
use inkwell::types::{AnyType, BasicTypeEnum};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, CallSiteValue, FunctionValue, IntValue, PointerValue,
    StructValue,
};

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn add_sret_callsite_attr(
        &self,
        call: CallSiteValue<'static>,
        ret_ty: BasicTypeEnum<'static>,
    ) {
        // Indirect calls do not inherit parameter attributes from a declaration,
        // so stamp sret on the callsite itself whenever we materialize the hidden slot.
        let sret_attr = self.context.create_type_attribute(
            Attribute::get_named_enum_kind_id("sret"),
            ret_ty.as_any_type_enum(),
        );
        call.add_attribute(AttributeLoc::Param(0), sret_attr);
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

    /// Call a function that uses sret convention (struct return via pointer).
    /// On Windows, alloca a result slot, pass as first arg, call, load result.
    /// On other targets, call normally and extract the return value.
    pub(crate) fn call_with_sret(
        &mut self,
        fn_val: FunctionValue<'static>,
        args: &[BasicMetadataValueEnum<'static>],
        ret_ty: BasicTypeEnum<'static>,
        name: &str,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        if self.target_is_windows() {
            let result_ptr = self
                .builder
                .build_alloca(ret_ty, "sret_slot")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            self.align_alloca(result_ptr, ret_ty)?;
            let mut all_args: Vec<BasicMetadataValueEnum<'static>> = vec![result_ptr.into()];
            all_args.extend_from_slice(args);
            let call = self
                .builder
                .build_call(fn_val, &all_args, "")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            call.set_call_convention(fn_val.get_call_conventions());
            self.add_sret_callsite_attr(call, ret_ty);
            self.builder
                .build_load(ret_ty, result_ptr, name)
                .map_err(|e| CodegenError::LlvmError(e.to_string()))
        } else {
            let call = self
                .builder
                .build_call(fn_val, args, name)
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            call.set_call_convention(fn_val.get_call_conventions());
            call.try_as_basic_value()
                .basic()
                .ok_or_else(|| CodegenError::LlvmError(format!("{} returned void", name)))
        }
    }

    /// Call a runtime function that returns %__aelys_string via sret.
    pub(crate) fn call_sret_returning_fn(
        &mut self,
        fn_val: FunctionValue<'static>,
        args: &[BasicMetadataValueEnum<'static>],
        name: &str,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        self.call_with_sret(fn_val, args, aelys_string_type(self.context).into(), name)
    }
}

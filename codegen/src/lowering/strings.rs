use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::lowering::stmts::RC_HEADER_SIZE;
use crate::types::aelys_string_type;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::context::Context;
use inkwell::module::Linkage;
use inkwell::types::{AnyType, BasicTypeEnum, StructType};
use inkwell::values::{
    BasicMetadataValueEnum, BasicValueEnum, CallSiteValue, FunctionValue, IntValue, PointerValue,
    StructValue,
};

/// a saturated refcount never decrements, which is what lets a release land on rodata
pub(crate) const IMMORTAL_REFCOUNT: u64 = u32::MAX as u64;
/// string bytes carry no child pointer, so the cycle collector must never trace them
pub(crate) const RC_FLAG_NO_TRACE: u64 = 0x02;
/// index of the byte array inside the headered constant, not a byte offset
const RC_HEADERED_DATA_FIELD: u32 = 7;

/// flags is its own byte: folded into a wider word it would only be right on little-endian
pub(crate) fn rc_headered_bytes_type(
    context: &'static Context,
    array_len: u32,
) -> StructType<'static> {
    let i8_ty = context.i8_type();
    let i32_ty = context.i32_type();
    context.struct_type(
        &[
            i32_ty.into(),
            i8_ty.into(),
            i8_ty.into(),
            i8_ty.into(),
            i8_ty.into(),
            i32_ty.into(),
            i32_ty.into(),
            i8_ty.array_type(array_len).into(),
        ],
        false,
    )
}

pub(crate) fn rc_headered_bytes_value(
    context: &'static Context,
    bytes: &[u8],
) -> StructValue<'static> {
    let i8_ty = context.i8_type();
    let i32_ty = context.i32_type();
    let mut data: Vec<_> = bytes
        .iter()
        .map(|byte| i8_ty.const_int(u64::from(*byte), false))
        .collect();
    data.push(i8_ty.const_zero());
    context.const_struct(
        &[
            i32_ty.const_int(IMMORTAL_REFCOUNT, false).into(),
            i8_ty.const_int(RC_FLAG_NO_TRACE, false).into(),
            i8_ty.const_zero().into(),
            i8_ty.const_zero().into(),
            i8_ty.const_zero().into(),
            i32_ty.const_zero().into(),
            i32_ty.const_zero().into(),
            i8_ty.const_array(&data).into(),
        ],
        false,
    )
}

/// walks struct field then array element, so the result points at the first byte and not at the array
pub(crate) fn rc_headered_data_indices(context: &'static Context) -> [IntValue<'static>; 3] {
    let i32_ty = context.i32_type();
    [
        i32_ty.const_zero(),
        i32_ty.const_int(u64::from(RC_HEADERED_DATA_FIELD), false),
        i32_ty.const_zero(),
    ]
}

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn add_sret_callsite_attr(
        &self,
        call: CallSiteValue<'static>,
        ret_ty: BasicTypeEnum<'static>,
    ) {
        // indirect calls do not inherit parameter attributes from a declaration,
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
        let text_len = u64::try_from(text.len()).map_err(|_| {
            CodegenError::UnsupportedInstruction("string literal too large".to_string())
        })?;
        let array_len = u32::try_from(text.len())
            .ok()
            .and_then(|n| n.checked_add(1))
            .ok_or_else(|| {
                CodegenError::UnsupportedInstruction("string literal too large".to_string())
            })?;

        let headered_ty = rc_headered_bytes_type(self.context, array_len);

        let global_ptr = if let Some(existing) = self.string_globals.get(text).copied() {
            existing
        } else {
            let name = format!("str_{}_{}", self.air_function.id.0, self.string_id);
            self.string_id = self.string_id.saturating_add(1);

            let global = self.module.add_global(headered_ty, None, &name);
            global.set_linkage(Linkage::Private);
            global.set_constant(true);
            global.set_alignment(RC_HEADER_SIZE as u32);
            global.set_initializer(&rc_headered_bytes_value(self.context, text.as_bytes()));
            let ptr = global.as_pointer_value();
            self.string_globals.insert(text.to_string(), ptr);
            ptr
        };

        let ptr = unsafe {
            self.builder.build_in_bounds_gep(
                headered_ty,
                global_ptr,
                &rc_headered_data_indices(self.context),
                "str_ptr",
            )
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

    pub(crate) fn call_sret_returning_fn(
        &mut self,
        fn_val: FunctionValue<'static>,
        args: &[BasicMetadataValueEnum<'static>],
        name: &str,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        self.call_with_sret(fn_val, args, aelys_string_type(self.context).into(), name)
    }
}

use crate::CodegenError;
use aelys_air::AirType;
use inkwell::AddressSpace;
use inkwell::types::{
    AnyTypeEnum, BasicMetadataTypeEnum, BasicType, BasicTypeEnum, PointerType, StructType,
};

const AELYS_STRING_STRUCT_NAME: &str = "__aelys_string";

pub fn air_type_to_llvm<'ctx>(
    ty: &AirType,
    context: &'ctx inkwell::context::Context,
) -> Result<AnyTypeEnum<'ctx>, CodegenError> {
    match ty {
        AirType::I8 | AirType::U8 => Ok(context.i8_type().into()),
        AirType::I16 | AirType::U16 => Ok(context.i16_type().into()),
        AirType::I32 | AirType::U32 => Ok(context.i32_type().into()),
        AirType::I64 | AirType::U64 => Ok(context.i64_type().into()),
        AirType::F32 => Ok(context.f32_type().into()),
        AirType::F64 => Ok(context.f64_type().into()),
        AirType::Bool => Ok(context.bool_type().into()),
        AirType::Str => Ok(aelys_string_type(context).into()),
        AirType::Ptr(inner) => Ok(pointer_to_air_type(inner, context)?.into()),
        AirType::Enum(name) => {
            // Data enums have a named struct type registered; simple enums use i32.
            let enum_struct_name = format!("__aelys_enum_{}", name);
            if let Some(st) = context.get_struct_type(&enum_struct_name) {
                Ok(st.into())
            } else {
                Ok(context.i32_type().into())
            }
        }
        AirType::Struct(name) => context
            .get_struct_type(name)
            .map(Into::into)
            .ok_or_else(|| CodegenError::UnsupportedType(format!("unknown struct type: {}", name))),
        AirType::Array(inner, len) => {
            let inner_ty = air_basic_type_to_llvm(inner, context)?;
            let len_u32 = u32::try_from(*len).map_err(|_| {
                CodegenError::UnsupportedType(format!("array length too large for LLVM: {}", len))
            })?;
            Ok(inner_ty.array_type(len_u32).into())
        }
        AirType::Slice(inner) => {
            let ptr_ty = pointer_to_air_type(inner, context);
            Ok(context
                .struct_type(&[ptr_ty?.into(), context.i64_type().into()], false)
                .into())
        }
        AirType::Vec(inner) => {
            let ptr_ty = pointer_to_air_type(inner, context)?;
            Ok(context
                .struct_type(
                    &[
                        ptr_ty.into(),
                        context.i64_type().into(),
                        context.i64_type().into(),
                    ],
                    false,
                )
                .into())
        }
        AirType::FnPtr {
            params,
            ret,
            conv,
        } => {
            if matches!(conv, aelys_air::CallingConv::Aelys) {
                // Aelys function values are fat pointers { fn_ptr, env_ptr },
                // regardless of whether they capture anything.
                //
                // This makes the representation uniform at call sites.
                // See the comment block in codegen/src/lowering/functions.rs if you wanna see everything
                Ok(closure_fat_ptr_type(context).into())
            } else {
                // C/Rust convention: bare function pointer
                let mut llvm_params: Vec<BasicMetadataTypeEnum<'ctx>> =
                    Vec::with_capacity(params.len());
                for param in params {
                    llvm_params.push(air_basic_type_to_llvm(param, context)?.into());
                }
                let fn_ty = match ret.as_ref() {
                    AirType::Void => context.void_type().fn_type(&llvm_params, false),
                    _ => air_basic_type_to_llvm(ret, context)?.fn_type(&llvm_params, false),
                };
                #[allow(deprecated)]
                {
                    Ok(fn_ty.ptr_type(AddressSpace::default()).into())
                }
            }
        }
        AirType::Param(param) => Err(CodegenError::UnsupportedType(format!(
            "unresolved AIR type parameter: {:?}",
            param
        ))),
        AirType::Opaque => Err(CodegenError::UnsupportedType(
            "unresolved Dynamic type reached codegen (should have been resolved by monomorphization or rejected by validation)".to_string(),
        )),
        AirType::Void => Ok(context.void_type().into()),
    }
}

pub fn air_basic_type_to_llvm<'ctx>(
    ty: &AirType,
    context: &'ctx inkwell::context::Context,
) -> Result<BasicTypeEnum<'ctx>, CodegenError> {
    match air_type_to_llvm(ty, context)? {
        AnyTypeEnum::ArrayType(t) => Ok(t.into()),
        AnyTypeEnum::FloatType(t) => Ok(t.into()),
        AnyTypeEnum::IntType(t) => Ok(t.into()),
        AnyTypeEnum::PointerType(t) => Ok(t.into()),
        AnyTypeEnum::StructType(t) => Ok(t.into()),
        AnyTypeEnum::VectorType(t) => Ok(t.into()),
        AnyTypeEnum::ScalableVectorType(t) => Ok(t.into()),
        AnyTypeEnum::FunctionType(_) => Err(CodegenError::UnsupportedType(
            "function type is not a basic LLVM type".to_string(),
        )),
        AnyTypeEnum::VoidType(_) => Err(CodegenError::UnsupportedType(
            "void type is not a basic LLVM type".to_string(),
        )),
    }
}

pub fn alignment_of(ty: BasicTypeEnum<'_>) -> u32 {
    match ty {
        BasicTypeEnum::IntType(int_ty) => int_alignment(int_ty.get_bit_width()),
        BasicTypeEnum::FloatType(float_ty) => float_alignment(float_ty.get_bit_width()),
        BasicTypeEnum::PointerType(_) => 8,
        BasicTypeEnum::ArrayType(array_ty) => alignment_of(array_ty.get_element_type()),
        BasicTypeEnum::StructType(struct_ty) => struct_alignment(struct_ty),
        BasicTypeEnum::VectorType(vector_ty) => alignment_of(vector_ty.get_element_type()),
        BasicTypeEnum::ScalableVectorType(vector_ty) => alignment_of(vector_ty.get_element_type()),
    }
}

fn int_alignment(bit_width: u32) -> u32 {
    match bit_width {
        0..=8 => 1,
        9..=16 => 2,
        17..=32 => 4,
        _ => 8,
    }
}

fn float_alignment(bit_width: u32) -> u32 {
    match bit_width {
        0..=16 => 2,
        17..=32 => 4,
        _ => 8,
    }
}

fn struct_alignment(ty: inkwell::types::StructType<'_>) -> u32 {
    if ty.is_opaque() {
        return 1;
    }
    if ty.is_packed() {
        return 1;
    }
    ty.get_field_types()
        .into_iter()
        .map(alignment_of)
        .max()
        .unwrap_or(1)
}

/// Fat pointer type for Aelys closures: `{ ptr fn_ptr, ptr env_ptr }`.
pub fn closure_fat_ptr_type(context: &'_ inkwell::context::Context) -> StructType<'_> {
    let ptr_ty = context.ptr_type(AddressSpace::default());
    context.struct_type(&[ptr_ty.into(), ptr_ty.into()], false)
}

fn pointer_to_i8<'ctx>(context: &'ctx inkwell::context::Context) -> PointerType<'ctx> {
    #[allow(deprecated)]
    {
        context.i8_type().ptr_type(AddressSpace::default())
    }
}

pub(crate) fn aelys_string_type<'ctx>(
    context: &'ctx inkwell::context::Context,
) -> StructType<'ctx> {
    if let Some(existing) = context.get_struct_type(AELYS_STRING_STRUCT_NAME) {
        if existing.is_opaque() {
            existing.set_body(
                &[pointer_to_i8(context).into(), context.i64_type().into()],
                false,
            );
        }
        return existing;
    }

    let ty = context.opaque_struct_type(AELYS_STRING_STRUCT_NAME);
    ty.set_body(
        &[pointer_to_i8(context).into(), context.i64_type().into()],
        false,
    );
    ty
}

fn pointer_to_air_type<'ctx>(
    ty: &AirType,
    context: &'ctx inkwell::context::Context,
) -> Result<PointerType<'ctx>, CodegenError> {
    if matches!(ty, AirType::Void) {
        return Ok(context.ptr_type(AddressSpace::default()));
    }

    #[allow(deprecated)]
    {
        Ok(air_basic_type_to_llvm(ty, context)?.ptr_type(AddressSpace::default()))
    }
}

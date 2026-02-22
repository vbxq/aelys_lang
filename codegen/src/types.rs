use crate::CodegenError;
use aelys_air::AirType;
use inkwell::AddressSpace;
use inkwell::types::{AnyTypeEnum, BasicMetadataTypeEnum, BasicType, BasicTypeEnum, PointerType};

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
        AirType::Str => Ok(pointer_to_i8(context).into()),
        AirType::Ptr(inner) => Ok(pointer_to_air_type(inner, context)?.into()),
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
        AirType::FnPtr {
            params,
            ret,
            conv: _,
        } => {
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
        AirType::Param(param) => Err(CodegenError::UnsupportedType(format!(
            "unresolved AIR type parameter: {:?}",
            param
        ))),
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

fn pointer_to_i8<'ctx>(context: &'ctx inkwell::context::Context) -> PointerType<'ctx> {
    #[allow(deprecated)]
    {
        context.i8_type().ptr_type(AddressSpace::default())
    }
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

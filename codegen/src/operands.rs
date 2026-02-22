use crate::CodegenError;
use crate::body::FunctionCodegen;
use crate::types::air_basic_type_to_llvm;
use aelys_air::{AirConst, AirFloatSize, AirIntSize, AirType, Operand};
use inkwell::AddressSpace;
use inkwell::context::Context;
use inkwell::types::IntType;
use inkwell::values::BasicValueEnum;

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn generate_operand(
        &mut self,
        operand: &Operand,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        match operand {
            Operand::Copy(id) | Operand::Move(id) => self.load_local(*id),
            Operand::Const(constant) => self.generate_const(constant),
        }
    }

    fn generate_const(&mut self, constant: &AirConst) -> Result<BasicValueEnum<'static>, CodegenError> {
        match constant {
            AirConst::IntLiteral(v) => Ok(self.context.i64_type().const_int(*v as u64, true).into()),
            AirConst::Int(v, size) => {
                let int_ty = int_type_for_size(self.context, *size);
                Ok(int_ty.const_int(*v as u64, is_signed_int_size(*size)).into())
            }
            AirConst::Float(v, AirFloatSize::F32) => Ok(self.context.f32_type().const_float(*v).into()),
            AirConst::Float(v, AirFloatSize::F64) => Ok(self.context.f64_type().const_float(*v).into()),
            AirConst::Bool(v) => Ok(self.context.bool_type().const_int(u64::from(*v), false).into()),
            AirConst::Str(s) => Ok(self.global_string_ptr(s)?.into()),
            AirConst::Null => Ok(self.context.ptr_type(AddressSpace::default()).const_null().into()),
            AirConst::ZeroInit(ty) => Ok(air_basic_type_to_llvm(ty, self.context)?.const_zero()),
            AirConst::Undef(ty) => Ok(air_basic_type_to_llvm(ty, self.context)?.const_zero()),
        }
    }

    pub(crate) fn operand_type(&self, operand: &Operand) -> Result<AirType, CodegenError> {
        match operand {
            Operand::Copy(id) | Operand::Move(id) => Ok(self.local_air_type(*id)?.clone()),
            Operand::Const(AirConst::IntLiteral(_)) => Ok(AirType::I64),
            Operand::Const(AirConst::Int(_, size)) => Ok(int_type_from_size(*size)),
            Operand::Const(AirConst::Float(_, AirFloatSize::F32)) => Ok(AirType::F32),
            Operand::Const(AirConst::Float(_, AirFloatSize::F64)) => Ok(AirType::F64),
            Operand::Const(AirConst::Bool(_)) => Ok(AirType::Bool),
            Operand::Const(AirConst::Str(_)) => Ok(AirType::Str),
            Operand::Const(AirConst::Null) => Ok(AirType::Ptr(Box::new(AirType::Void))),
            Operand::Const(AirConst::ZeroInit(ty)) | Operand::Const(AirConst::Undef(ty)) => {
                Ok(ty.clone())
            }
        }
    }
}

pub(crate) fn int_type_for_size(context: &'static Context, size: AirIntSize) -> IntType<'static> {
    match size {
        AirIntSize::I8 | AirIntSize::U8 => context.i8_type(),
        AirIntSize::I16 | AirIntSize::U16 => context.i16_type(),
        AirIntSize::I32 | AirIntSize::U32 => context.i32_type(),
        AirIntSize::I64 | AirIntSize::U64 => context.i64_type(),
    }
}

pub(crate) fn int_type_from_size(size: AirIntSize) -> AirType {
    match size {
        AirIntSize::I8 => AirType::I8,
        AirIntSize::I16 => AirType::I16,
        AirIntSize::I32 => AirType::I32,
        AirIntSize::I64 => AirType::I64,
        AirIntSize::U8 => AirType::U8,
        AirIntSize::U16 => AirType::U16,
        AirIntSize::U32 => AirType::U32,
        AirIntSize::U64 => AirType::U64,
    }
}

pub(crate) fn is_signed_int_size(size: AirIntSize) -> bool {
    matches!(
        size,
        AirIntSize::I8 | AirIntSize::I16 | AirIntSize::I32 | AirIntSize::I64
    )
}

pub(crate) fn constant_kind_name(c: &AirConst) -> &'static str {
    match c {
        AirConst::IntLiteral(_) => "IntLiteral",
        AirConst::Int(_, _) => "Int",
        AirConst::Float(_, _) => "Float",
        AirConst::Bool(_) => "Bool",
        AirConst::Str(_) => "Str",
        AirConst::Null => "Null",
        AirConst::ZeroInit(_) => "ZeroInit",
        AirConst::Undef(_) => "Undef",
    }
}

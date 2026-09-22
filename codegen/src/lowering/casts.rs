use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use aelys_air::{AirType, Operand};
use inkwell::context::Context;
use inkwell::intrinsics::Intrinsic;
use inkwell::types::{FloatType, IntType};
use inkwell::values::{BasicValueEnum, FloatValue};

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn generate_cast(
        &mut self,
        operand: &Operand,
        from: &AirType,
        to: &AirType,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let value = self.generate_operand(operand)?;

        if from == to {
            return Ok(value);
        }

        if let (Some((from_bits, from_signed)), Some((to_bits, _))) = (int_info(from), int_info(to))
        {
            let target_ty = int_type_for_air(self.context, to)?;
            if from_bits > to_bits {
                return self
                    .builder
                    .build_int_truncate(value.into_int_value(), target_ty, "trunc")
                    .map(Into::into)
                    .map_err(|e| CodegenError::LlvmError(e.to_string()));
            }

            if from_bits < to_bits {
                if from_signed {
                    return self
                        .builder
                        .build_int_s_extend(value.into_int_value(), target_ty, "sext")
                        .map(Into::into)
                        .map_err(|e| CodegenError::LlvmError(e.to_string()));
                }
                return self
                    .builder
                    .build_int_z_extend(value.into_int_value(), target_ty, "zext")
                    .map(Into::into)
                    .map_err(|e| CodegenError::LlvmError(e.to_string()));
            }

            return Ok(value);
        }

        if let (Some(from_bits), Some(to_bits)) = (float_info(from), float_info(to)) {
            let target_ty = float_type_for_air(self.context, to)?;
            if from_bits > to_bits {
                return self
                    .builder
                    .build_float_trunc(value.into_float_value(), target_ty, "fptrunc")
                    .map(Into::into)
                    .map_err(|e| CodegenError::LlvmError(e.to_string()));
            }

            if from_bits < to_bits {
                return self
                    .builder
                    .build_float_ext(value.into_float_value(), target_ty, "fpext")
                    .map(Into::into)
                    .map_err(|e| CodegenError::LlvmError(e.to_string()));
            }

            return Ok(value);
        }

        if float_info(from).is_some() && int_info(to).is_some() {
            let target_ty = int_type_for_air(self.context, to)?;
            let (_, to_signed) = int_info(to).unwrap();
            return self.build_saturating_float_to_int(
                value.into_float_value(),
                target_ty,
                to_signed,
            );
        }

        if int_info(from).is_some() && float_info(to).is_some() {
            let target_ty = float_type_for_air(self.context, to)?;
            let (_, from_signed) = int_info(from).unwrap();
            return if from_signed {
                self.builder
                    .build_signed_int_to_float(value.into_int_value(), target_ty, "sitofp")
                    .map(Into::into)
            } else {
                self.builder
                    .build_unsigned_int_to_float(value.into_int_value(), target_ty, "uitofp")
                    .map(Into::into)
            }
            .map_err(|e| CodegenError::LlvmError(e.to_string()));
        }

        Err(CodegenError::UnsupportedInstruction(format!(
            "unsupported cast {:?} -> {:?}",
            from, to
        )))
    }

    // a bare fptosi/fptoui whose operand does not fit the target is poison, so the answer would change with the optimizer
    fn build_saturating_float_to_int(
        &mut self,
        value: FloatValue<'static>,
        target_ty: IntType<'static>,
        signed: bool,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let name = if signed {
            "llvm.fptosi.sat"
        } else {
            "llvm.fptoui.sat"
        };
        let intrinsic = Intrinsic::find(name)
            .ok_or_else(|| CodegenError::LlvmError(format!("{name} is not a known intrinsic")))?;
        let declaration = intrinsic
            .get_declaration(self.module, &[target_ty.into(), value.get_type().into()])
            .ok_or_else(|| {
                CodegenError::LlvmError(format!(
                    "{name} has no declaration for {:?} -> {:?}",
                    value.get_type(),
                    target_ty
                ))
            })?;
        self.builder
            .build_call(declaration, &[value.into()], "fptoint_sat")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| CodegenError::LlvmError(format!("{name} produced no value")))
    }
}

fn int_type_for_air(
    context: &'static Context,
    ty: &AirType,
) -> Result<IntType<'static>, CodegenError> {
    match ty {
        AirType::I8 | AirType::U8 => Ok(context.i8_type()),
        AirType::I16 | AirType::U16 => Ok(context.i16_type()),
        AirType::I32 | AirType::U32 | AirType::Char => Ok(context.i32_type()),
        AirType::I64 | AirType::U64 => Ok(context.i64_type()),
        AirType::Bool => Ok(context.bool_type()),
        _ => Err(CodegenError::UnsupportedType(format!(
            "expected int type, got {:?}",
            ty
        ))),
    }
}

fn float_type_for_air(
    context: &'static Context,
    ty: &AirType,
) -> Result<FloatType<'static>, CodegenError> {
    match ty {
        AirType::F32 => Ok(context.f32_type()),
        AirType::F64 => Ok(context.f64_type()),
        _ => Err(CodegenError::UnsupportedType(format!(
            "expected float type, got {:?}",
            ty
        ))),
    }
}

fn int_info(ty: &AirType) -> Option<(u32, bool)> {
    match ty {
        AirType::I8 => Some((8, true)),
        AirType::I16 => Some((16, true)),
        AirType::I32 => Some((32, true)),
        AirType::I64 => Some((64, true)),
        AirType::U8 => Some((8, false)),
        AirType::U16 => Some((16, false)),
        AirType::U32 => Some((32, false)),
        AirType::U64 => Some((64, false)),
        AirType::Char => Some((32, false)),
        AirType::Bool => Some((1, false)),
        _ => None,
    }
}

fn float_info(ty: &AirType) -> Option<u32> {
    match ty {
        AirType::F32 => Some(32),
        AirType::F64 => Some(64),
        _ => None,
    }
}

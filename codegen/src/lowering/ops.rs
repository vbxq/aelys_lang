use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use aelys_air::{AirType, BinOp, Operand, UnOp};
use inkwell::values::{BasicValueEnum, FloatValue, IntValue};
use inkwell::{FloatPredicate, IntPredicate};

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn generate_binary_op(
        &mut self,
        op: &BinOp,
        left: &Operand,
        right: &Operand,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let left_val = self.generate_operand(left)?;
        let right_val = self.generate_operand(right)?;

        if left_val.is_int_value() && right_val.is_int_value() {
            return self.generate_int_binary_op(
                op.clone(),
                left_val.into_int_value(),
                right_val.into_int_value(),
            );
        }

        if left_val.is_float_value() && right_val.is_float_value() {
            return self.generate_float_binary_op(
                op.clone(),
                left_val.into_float_value(),
                right_val.into_float_value(),
            );
        }

        let left_ty = self.operand_type(left)?;
        if matches!(left_ty, AirType::Str) {
            let right_ty = self.operand_type(right)?;
            if matches!(right_ty, AirType::Str) {
                return self.generate_string_binary_op(
                    op.clone(),
                    left_val.into_struct_value(),
                    right_val.into_struct_value(),
                );
            }
        }

        Err(CodegenError::UnsupportedInstruction(
            "binary op with non int/float operands".to_string(),
        ))
    }

    fn generate_int_binary_op(
        &mut self,
        op: BinOp,
        left: IntValue<'static>,
        right: IntValue<'static>,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let value = match op {
            BinOp::Add => self
                .builder
                .build_int_add(left, right, "iadd")
                .map(Into::into),
            BinOp::Sub => self
                .builder
                .build_int_sub(left, right, "isub")
                .map(Into::into),
            BinOp::Mul => self
                .builder
                .build_int_mul(left, right, "imul")
                .map(Into::into),
            BinOp::Div => self
                .builder
                .build_int_signed_div(left, right, "isdiv")
                .map(Into::into),
            BinOp::Rem => self
                .builder
                .build_int_signed_rem(left, right, "isrem")
                .map(Into::into),
            BinOp::Eq => self
                .builder
                .build_int_compare(IntPredicate::EQ, left, right, "icmp_eq")
                .map(Into::into),
            BinOp::Ne => self
                .builder
                .build_int_compare(IntPredicate::NE, left, right, "icmp_ne")
                .map(Into::into),
            BinOp::Lt => self
                .builder
                .build_int_compare(IntPredicate::SLT, left, right, "icmp_lt")
                .map(Into::into),
            BinOp::Le => self
                .builder
                .build_int_compare(IntPredicate::SLE, left, right, "icmp_le")
                .map(Into::into),
            BinOp::Gt => self
                .builder
                .build_int_compare(IntPredicate::SGT, left, right, "icmp_gt")
                .map(Into::into),
            BinOp::Ge => self
                .builder
                .build_int_compare(IntPredicate::SGE, left, right, "icmp_ge")
                .map(Into::into),
            BinOp::And | BinOp::BitAnd => {
                self.builder.build_and(left, right, "iand").map(Into::into)
            }
            BinOp::Or | BinOp::BitOr => self.builder.build_or(left, right, "ior").map(Into::into),
            BinOp::BitXor => self.builder.build_xor(left, right, "ixor").map(Into::into),
            BinOp::Shl => self
                .builder
                .build_left_shift(left, right, "ishl")
                .map(Into::into),
            BinOp::Shr => self
                .builder
                .build_right_shift(left, right, true, "ishr")
                .map(Into::into),
            BinOp::CheckedAdd => {
                return Err(self.unsupported_air(
                    "BinOp::CheckedAdd",
                    "checked integer add is not implemented for LLVM backend",
                ));
            }
            BinOp::CheckedSub => {
                return Err(self.unsupported_air(
                    "BinOp::CheckedSub",
                    "checked integer sub is not implemented for LLVM backend",
                ));
            }
            BinOp::CheckedMul => {
                return Err(self.unsupported_air(
                    "BinOp::CheckedMul",
                    "checked integer mul is not implemented for LLVM backend",
                ));
            }
        };

        value.map_err(|e| CodegenError::LlvmError(e.to_string()))
    }

    fn generate_float_binary_op(
        &mut self,
        op: BinOp,
        left: FloatValue<'static>,
        right: FloatValue<'static>,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let value = match op {
            BinOp::Add => self
                .builder
                .build_float_add(left, right, "fadd")
                .map(Into::into),
            BinOp::Sub => self
                .builder
                .build_float_sub(left, right, "fsub")
                .map(Into::into),
            BinOp::Mul => self
                .builder
                .build_float_mul(left, right, "fmul")
                .map(Into::into),
            BinOp::Div => self
                .builder
                .build_float_div(left, right, "fdiv")
                .map(Into::into),
            BinOp::Rem => self
                .builder
                .build_float_rem(left, right, "frem")
                .map(Into::into),
            BinOp::Eq => self
                .builder
                .build_float_compare(FloatPredicate::OEQ, left, right, "fcmp_eq")
                .map(Into::into),
            BinOp::Ne => self
                .builder
                .build_float_compare(FloatPredicate::ONE, left, right, "fcmp_ne")
                .map(Into::into),
            BinOp::Lt => self
                .builder
                .build_float_compare(FloatPredicate::OLT, left, right, "fcmp_lt")
                .map(Into::into),
            BinOp::Le => self
                .builder
                .build_float_compare(FloatPredicate::OLE, left, right, "fcmp_le")
                .map(Into::into),
            BinOp::Gt => self
                .builder
                .build_float_compare(FloatPredicate::OGT, left, right, "fcmp_gt")
                .map(Into::into),
            BinOp::Ge => self
                .builder
                .build_float_compare(FloatPredicate::OGE, left, right, "fcmp_ge")
                .map(Into::into),
            _ => {
                return Err(CodegenError::UnsupportedInstruction(
                    "unsupported float binop".to_string(),
                ));
            }
        };

        value.map_err(|e| CodegenError::LlvmError(e.to_string()))
    }

    pub(crate) fn generate_unary_op(
        &mut self,
        op: &UnOp,
        operand: &Operand,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let value = self.generate_operand(operand)?;

        match op {
            UnOp::Neg => {
                if value.is_int_value() {
                    return self
                        .builder
                        .build_int_neg(value.into_int_value(), "ineg")
                        .map(Into::into)
                        .map_err(|e| CodegenError::LlvmError(e.to_string()));
                }

                if value.is_float_value() {
                    return self
                        .builder
                        .build_float_neg(value.into_float_value(), "fneg")
                        .map(Into::into)
                        .map_err(|e| CodegenError::LlvmError(e.to_string()));
                }
            }
            UnOp::Not => {
                if matches!(self.operand_type(operand)?, AirType::Bool) {
                    return self
                        .builder
                        .build_not(value.into_int_value(), "not")
                        .map(Into::into)
                        .map_err(|e| CodegenError::LlvmError(e.to_string()));
                }
            }
            UnOp::BitNot => {
                if value.is_int_value() {
                    return self
                        .builder
                        .build_not(value.into_int_value(), "bitnot")
                        .map(Into::into)
                        .map_err(|e| CodegenError::LlvmError(e.to_string()));
                }
            }
        }

        Err(CodegenError::UnsupportedInstruction(
            "unsupported unary op".to_string(),
        ))
    }

    fn generate_string_binary_op(
        &mut self,
        op: BinOp,
        left: inkwell::values::StructValue<'static>,
        right: inkwell::values::StructValue<'static>,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        match op {
            BinOp::Eq => {
                let str_eq_fn = self.ensure_str_eq_function();
                let result = self
                    .builder
                    .build_call(str_eq_fn, &[left.into(), right.into()], "str_eq")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                result
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| CodegenError::LlvmError("str_eq returned void".to_string()))
            }
            BinOp::Ne => {
                let str_eq_fn = self.ensure_str_eq_function();
                let eq_result = self
                    .builder
                    .build_call(str_eq_fn, &[left.into(), right.into()], "str_eq")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                let eq_val = eq_result
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| CodegenError::LlvmError("str_eq returned void".to_string()))?
                    .into_int_value();
                self.builder
                    .build_not(eq_val, "str_ne")
                    .map(Into::into)
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))
            }
            _ => Err(CodegenError::UnsupportedInstruction(
                "unsupported string binary op (only == and != are supported)".to_string()
            )),
        }
    }
}

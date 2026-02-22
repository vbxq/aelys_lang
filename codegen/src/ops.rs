use crate::CodegenError;
use crate::body::FunctionCodegen;
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
            BinOp::Add => self.builder.build_int_add(left, right, "iadd").map(Into::into),
            BinOp::Sub => self.builder.build_int_sub(left, right, "isub").map(Into::into),
            BinOp::Mul => self.builder.build_int_mul(left, right, "imul").map(Into::into),
            BinOp::Div => self.builder.build_int_signed_div(left, right, "isdiv").map(Into::into),
            BinOp::Rem => self.builder.build_int_signed_rem(left, right, "isrem").map(Into::into),
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
            BinOp::And | BinOp::BitAnd => self.builder.build_and(left, right, "iand").map(Into::into),
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
            BinOp::CheckedAdd | BinOp::CheckedSub | BinOp::CheckedMul => todo!(),
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
            BinOp::Add => self.builder.build_float_add(left, right, "fadd").map(Into::into),
            BinOp::Sub => self.builder.build_float_sub(left, right, "fsub").map(Into::into),
            BinOp::Mul => self.builder.build_float_mul(left, right, "fmul").map(Into::into),
            BinOp::Div => self.builder.build_float_div(left, right, "fdiv").map(Into::into),
            BinOp::Rem => self.builder.build_float_rem(left, right, "frem").map(Into::into),
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
}

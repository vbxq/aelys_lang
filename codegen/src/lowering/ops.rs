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
            let left_ty = self.operand_type(left)?;
            return self.generate_int_binary_op(
                op.clone(),
                left_val.into_int_value(),
                right_val.into_int_value(),
                &left_ty,
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

        let left_ty = self.operand_type(left)?;
        if matches!(left_ty, AirType::Rc(_))
            && matches!(op, BinOp::Eq | BinOp::Ne)
            && left_val.is_pointer_value()
            && right_val.is_pointer_value()
        {
            let i64t = self.context.i64_type();
            let l = self
                .builder
                .build_ptr_to_int(left_val.into_pointer_value(), i64t, "rc_l")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            let r = self
                .builder
                .build_ptr_to_int(right_val.into_pointer_value(), i64t, "rc_r")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            let pred = if matches!(op, BinOp::Eq) {
                IntPredicate::EQ
            } else {
                IntPredicate::NE
            };
            return self
                .builder
                .build_int_compare(pred, l, r, "rc_cmp")
                .map(Into::into)
                .map_err(|e| CodegenError::LlvmError(e.to_string()));
        }
        if let AirType::Struct(r) = &left_ty
            && aelys_air::tuples::is_tuple_name(&r.symbol())
            && matches!(op, BinOp::Eq | BinOp::Ne)
            && left_val.is_struct_value()
            && right_val.is_struct_value()
        {
            let eq = self.tuple_value_eq(
                &r.symbol(),
                left_val.into_struct_value(),
                right_val.into_struct_value(),
            )?;
            if matches!(op, BinOp::Eq) {
                return Ok(eq.into());
            }
            return self
                .builder
                .build_not(eq, "tuple_ne")
                .map(Into::into)
                .map_err(|e| CodegenError::LlvmError(e.to_string()));
        }
        if matches!(left_ty, AirType::Enum(_)) {
            return Err(CodegenError::UnsupportedInstruction(format!(
                "comparison on data enum type `{:?}` is not supported; use `match` instead",
                left_ty
            )));
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
        operand_ty: &AirType,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let is_unsigned = matches!(
            operand_ty,
            AirType::U8 | AirType::U16 | AirType::U32 | AirType::U64 | AirType::Char
        );

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
            BinOp::Div => {
                self.emit_div_zero_check(right)?;
                if is_unsigned {
                    self.builder
                        .build_int_unsigned_div(left, right, "iudiv")
                        .map(Into::into)
                } else {
                    self.emit_div_overflow_check(left, right)?;
                    self.builder
                        .build_int_signed_div(left, right, "isdiv")
                        .map(Into::into)
                }
            }
            BinOp::Rem => {
                self.emit_div_zero_check(right)?;
                if is_unsigned {
                    self.builder
                        .build_int_unsigned_rem(left, right, "iurem")
                        .map(Into::into)
                } else {
                    self.emit_div_overflow_check(left, right)?;
                    self.builder
                        .build_int_signed_rem(left, right, "isrem")
                        .map(Into::into)
                }
            }
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
                .build_int_compare(
                    if is_unsigned {
                        IntPredicate::ULT
                    } else {
                        IntPredicate::SLT
                    },
                    left,
                    right,
                    "icmp_lt",
                )
                .map(Into::into),
            BinOp::Le => self
                .builder
                .build_int_compare(
                    if is_unsigned {
                        IntPredicate::ULE
                    } else {
                        IntPredicate::SLE
                    },
                    left,
                    right,
                    "icmp_le",
                )
                .map(Into::into),
            BinOp::Gt => self
                .builder
                .build_int_compare(
                    if is_unsigned {
                        IntPredicate::UGT
                    } else {
                        IntPredicate::SGT
                    },
                    left,
                    right,
                    "icmp_gt",
                )
                .map(Into::into),
            BinOp::Ge => self
                .builder
                .build_int_compare(
                    if is_unsigned {
                        IntPredicate::UGE
                    } else {
                        IntPredicate::SGE
                    },
                    left,
                    right,
                    "icmp_ge",
                )
                .map(Into::into),
            BinOp::And | BinOp::BitAnd => {
                self.builder.build_and(left, right, "iand").map(Into::into)
            }
            BinOp::Or | BinOp::BitOr => self.builder.build_or(left, right, "ior").map(Into::into),
            BinOp::BitXor => self.builder.build_xor(left, right, "ixor").map(Into::into),
            BinOp::Shl => {
                let bitwidth = left.get_type().get_bit_width();
                let mask = left.get_type().const_int((bitwidth - 1) as u64, false);
                let masked = self
                    .builder
                    .build_and(right, mask, "shl_mask")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                self.builder
                    .build_left_shift(left, masked, "ishl")
                    .map(Into::into)
            }
            BinOp::Shr => {
                let bitwidth = left.get_type().get_bit_width();
                let mask = left.get_type().const_int((bitwidth - 1) as u64, false);
                let masked = self
                    .builder
                    .build_and(right, mask, "shr_mask")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                self.builder
                    .build_right_shift(left, masked, !is_unsigned, "ishr")
                    .map(Into::into)
            }
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
                .build_float_compare(FloatPredicate::UNE, left, right, "fcmp_ne")
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

    fn tuple_value_eq(
        &mut self,
        symbol: &str,
        left: inkwell::values::StructValue<'static>,
        right: inkwell::values::StructValue<'static>,
    ) -> Result<IntValue<'static>, CodegenError> {
        let field_tys: Vec<AirType> = self
            .program
            .structs
            .iter()
            .find(|s| s.name == symbol)
            .map(|s| s.fields.iter().map(|f| f.ty.clone()).collect())
            .ok_or_else(|| CodegenError::UnsupportedType(format!("unknown tuple `{symbol}`")))?;
        let mut acc = self.context.bool_type().const_int(1, false);
        for (i, ty) in field_tys.iter().enumerate() {
            let index = u32::try_from(i).map_err(|_| {
                CodegenError::UnsupportedType(format!("field index overflow on {symbol}"))
            })?;
            let l = self
                .builder
                .build_extract_value(left, index, "tuple_l")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            let r = self
                .builder
                .build_extract_value(right, index, "tuple_r")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            let eq: IntValue<'static> = match ty {
                AirType::Str => self
                    .generate_string_binary_op(BinOp::Eq, l.into_struct_value(), r.into_struct_value())?
                    .into_int_value(),
                AirType::Struct(inner) if aelys_air::tuples::is_tuple_name(&inner.symbol()) => {
                    self.tuple_value_eq(&inner.symbol(), l.into_struct_value(), r.into_struct_value())?
                }
                AirType::F32 | AirType::F64 => self
                    .builder
                    .build_float_compare(
                        FloatPredicate::OEQ,
                        l.into_float_value(),
                        r.into_float_value(),
                        "tuple_feq",
                    )
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?,
                AirType::I8
                | AirType::I16
                | AirType::I32
                | AirType::I64
                | AirType::U8
                | AirType::U16
                | AirType::U32
                | AirType::U64
                | AirType::Bool
                | AirType::Char => self
                    .builder
                    .build_int_compare(
                        IntPredicate::EQ,
                        l.into_int_value(),
                        r.into_int_value(),
                        "tuple_ieq",
                    )
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?,
                other => {
                    return Err(CodegenError::UnsupportedInstruction(format!(
                        "equality on a tuple element of type {other:?} is not supported"
                    )));
                }
            };
            acc = self
                .builder
                .build_and(acc, eq, "tuple_and")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        }
        Ok(acc)
    }

    fn generate_string_binary_op(
        &mut self,
        op: BinOp,
        left: inkwell::values::StructValue<'static>,
        right: inkwell::values::StructValue<'static>,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        match op {
            BinOp::Eq | BinOp::Ne => {
                // exctract ptr/len from both string structs (flat abi)
                let (a_ptr, a_len) = self.string_parts_from_value(left)?;
                let (b_ptr, b_len) = self.string_parts_from_value(right)?;

                let str_eq_fn = self.ensure_str_eq_function();
                let result = self
                    .builder
                    .build_call(
                        str_eq_fn,
                        &[a_ptr.into(), a_len.into(), b_ptr.into(), b_len.into()],
                        "str_eq",
                    )
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                let i64_val = result
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| CodegenError::LlvmError("str_eq returned void".to_string()))?
                    .into_int_value();

                let is_eq = matches!(op, BinOp::Eq);
                let pred = if is_eq {
                    IntPredicate::NE
                } else {
                    IntPredicate::EQ
                };
                let name = if is_eq { "str_eq" } else { "str_ne" };
                self.builder
                    .build_int_compare(pred, i64_val, self.context.i64_type().const_zero(), name)
                    .map(Into::into)
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))
            }
            BinOp::Lt | BinOp::Le | BinOp::Gt | BinOp::Ge => {
                let (a_ptr, a_len) = self.string_parts_from_value(left)?;
                let (b_ptr, b_len) = self.string_parts_from_value(right)?;
                let cmp_fn = self.ensure_str_cmp_function();
                let order = self
                    .builder
                    .build_call(
                        cmp_fn,
                        &[a_ptr.into(), a_len.into(), b_ptr.into(), b_len.into()],
                        "str_cmp",
                    )
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| CodegenError::LlvmError("str_cmp returned void".to_string()))?
                    .into_int_value();
                let pred = match op {
                    BinOp::Lt => IntPredicate::SLT,
                    BinOp::Le => IntPredicate::SLE,
                    BinOp::Gt => IntPredicate::SGT,
                    _ => IntPredicate::SGE,
                };
                self.builder
                    .build_int_compare(pred, order, self.context.i64_type().const_zero(), "str_ord")
                    .map(Into::into)
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))
            }
            BinOp::Add => self.emit_str_concat(left.into(), right.into()),
            _ => Err(CodegenError::UnsupportedInstruction(
                "unsupported string binary op (only +, ==, !=, <, <=, > and >= are supported)"
                    .to_string(),
            )),
        }
    }
}

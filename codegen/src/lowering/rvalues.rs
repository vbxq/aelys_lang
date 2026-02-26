use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::types::air_basic_type_to_llvm;
use aelys_air::{AirType, Operand, Rvalue};
use inkwell::values::{BasicValue, BasicValueEnum};

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn generate_rvalue(
        &mut self,
        rvalue: &Rvalue,
        expected_ty: Option<&AirType>,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        match rvalue {
            Rvalue::Use(operand) => self.generate_operand(operand),
            Rvalue::BinaryOp(op, left, right) => self.generate_binary_op(op, left, right),
            Rvalue::UnaryOp(op, operand) => self.generate_unary_op(op, operand),
            Rvalue::Call { func, args } => {
                self.generate_call(func, args, expected_ty)?.ok_or_else(|| {
                    CodegenError::LlvmError("call used as value returned void".to_string())
                })
            }
            Rvalue::StructInit { name, fields } => self.generate_struct_init(name, fields),
            Rvalue::FieldAccess { base, field } => self.generate_field_access(base, field),
            Rvalue::AddressOf(local) => Ok(self.lookup_local_ptr(*local)?.as_basic_value_enum()),
            Rvalue::Deref(operand) => {
                let ptr = self.generate_operand(operand)?.into_pointer_value();
                let inner = match self.operand_type(operand)? {
                    AirType::Ptr(inner) => *inner,
                    other => {
                        return Err(CodegenError::UnsupportedType(format!(
                            "cannot dereference operand of type {:?}",
                            other
                        )));
                    }
                };
                let inner_ty = air_basic_type_to_llvm(&inner, self.context)?;
                self.load_value(inner_ty, ptr, "deref")
            }
            Rvalue::Cast { operand, from, to } => self.generate_cast(operand, from, to),
            Rvalue::Discriminant(_) => Err(self.unsupported_air(
                "Rvalue::Discriminant",
                "discriminant extraction is not implemented for LLVM backend",
            )),
            Rvalue::Index { base, index } => self.generate_index(base, index),
        }
    }

    fn generate_index(
        &mut self,
        base: &Operand,
        index: &Operand,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let idx_val = self.generate_operand(index)?.into_int_value();
        let base_ty = self.operand_type(base)?;

        match base_ty {
            AirType::Array(ref inner, n) => {
                let length = self.context.i64_type().const_int(n, false);
                self.emit_bounds_check(idx_val, length)?;

                let base_local = match base {
                    Operand::Copy(id) | Operand::Move(id) => *id,
                    _ => {
                        return Err(CodegenError::LlvmError(
                            "array index base must be a local".to_string(),
                        ));
                    }
                };
                let arr_ty = air_basic_type_to_llvm(&base_ty, self.context)?;
                let ptr = self.lookup_local_ptr(base_local)?;
                let zero = self.context.i64_type().const_zero();
                let elem_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(arr_ty, ptr, &[zero, idx_val], "idx_elem_ptr")
                }
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                let elem_ty = air_basic_type_to_llvm(inner, self.context)?;
                self.load_value(elem_ty, elem_ptr, "idx_load")
            }
            AirType::Slice(ref inner) => {
                let slice_val = self.generate_operand(base)?.into_struct_value();
                let data_ptr = self
                    .builder
                    .build_extract_value(slice_val, 0, "slice_ptr")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?
                    .into_pointer_value();
                let length = self
                    .builder
                    .build_extract_value(slice_val, 1, "slice_len")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?
                    .into_int_value();
                self.emit_bounds_check(idx_val, length)?;
                let elem_ty = air_basic_type_to_llvm(inner, self.context)?;
                let elem_ptr = unsafe {
                    self.builder
                        .build_in_bounds_gep(elem_ty, data_ptr, &[idx_val], "idx_elem_ptr")
                }
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                self.load_value(elem_ty, elem_ptr, "idx_load")
            }
            AirType::Str => {
                // UTF-8 character indexing: delegate to runtime because
                // finding the n-th codepoint requires scanning byte boundaries.
                let str_val = self.generate_operand(base)?.into_struct_value();
                let (str_ptr, str_len) = self.string_parts_from_value(str_val)?;
                let char_at_fn = self.ensure_str_char_at_function();

                // Windows x64 MSVC uses sret for struct returns
                if self.target_is_windows() {
                    let string_ty = crate::types::aelys_string_type(self.context);
                    let result_ptr = self
                        .builder
                        .build_alloca(string_ty, "sret_slot")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                    self.align_alloca(result_ptr, string_ty.into())?;
                    self.builder
                        .build_call(
                            char_at_fn,
                            &[result_ptr.into(), str_ptr.into(), str_len.into(), idx_val.into()],
                            "",
                        )
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                    self.builder
                        .build_load(string_ty, result_ptr, "str_char_at")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))
                } else {
                    let result = self
                        .builder
                        .build_call(
                            char_at_fn,
                            &[str_ptr.into(), str_len.into(), idx_val.into()],
                            "str_char_at",
                        )
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                    result.try_as_basic_value().basic().ok_or_else(|| {
                        CodegenError::LlvmError("__aelys_str_char_at returned void".to_string())
                    })
                }
            }
            other => Err(CodegenError::UnsupportedType(format!(
                "cannot index into {:?}",
                other
            ))),
        }
    }
}

use crate::CodegenContext;
use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::types::air_basic_type_to_llvm;
use aelys_air::layout::{enum_has_data, enum_max_payload_size};
use aelys_air::{AirProgram, AirType, Operand};
use inkwell::types::StructType;
use inkwell::values::{BasicValueEnum, PointerValue};

impl CodegenContext {
    pub(crate) fn declare_struct_types(&self, program: &AirProgram) -> Result<(), CodegenError> {
        for struct_def in &program.structs {
            if self.context.get_struct_type(&struct_def.name).is_none() {
                self.context.opaque_struct_type(&struct_def.name);
            }
        }

        for struct_def in &program.structs {
            let llvm_struct = self
                .context
                .get_struct_type(&struct_def.name)
                .ok_or_else(|| {
                    CodegenError::LlvmError(format!(
                        "failed to retrieve declared struct: {}",
                        struct_def.name
                    ))
                })?;

            if llvm_struct.is_opaque() {
                let mut field_types = Vec::with_capacity(struct_def.fields.len());
                for field in &struct_def.fields {
                    field_types.push(air_basic_type_to_llvm(&field.ty, self.context)?);
                }
                llvm_struct.set_body(&field_types, false);
            }
        }

        // Declare named struct types for data enums: { i32, [N x i8] }
        for enum_def in &program.enums {
            if enum_has_data(enum_def) {
                let max_payload = enum_max_payload_size(enum_def, &program.struct_sizes);
                let enum_struct_name = format!("__aelys_enum_{}", enum_def.name);
                let enum_ty = self.context.opaque_struct_type(&enum_struct_name);
                let tag_ty = self.context.i32_type().into();
                let payload_ty = self.context.i8_type().array_type(max_payload).into();
                enum_ty.set_body(&[tag_ty, payload_ty], false);
            }
        }

        Ok(())
    }
}

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn generate_struct_init(
        &mut self,
        name: &str,
        fields: &[(String, Operand)],
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let struct_ty = self
            .context
            .get_struct_type(name)
            .ok_or_else(|| CodegenError::UnsupportedType(format!("unknown struct {}", name)))?;
        let tmp = self
            .builder
            .build_alloca(struct_ty, "struct_tmp")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        self.align_alloca(tmp, struct_ty.into())?;

        for (field_name, operand) in fields {
            let index = self.struct_field_index(name, field_name)?;
            let ptr = self
                .builder
                .build_struct_gep(struct_ty, tmp, index, "field_ptr")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            let value = self.generate_operand(operand)?;
            self.store_value(ptr, value)?;
        }

        self.load_value(struct_ty.into(), tmp, "struct_value")
    }

    pub(crate) fn generate_field_access(
        &mut self,
        base: &Operand,
        field: &str,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        if matches!(self.operand_type(base)?, AirType::Str) {
            if field != "len" {
                return Err(CodegenError::UnsupportedType(format!(
                    "unknown field `{}` on `Str`",
                    field
                )));
            }
            let str_value = self.generate_operand(base)?;
            if !str_value.is_struct_value() {
                return Err(CodegenError::UnsupportedType(
                    "expected Str fat pointer value for field access".to_string(),
                ));
            }
            return Ok(self
                .builder
                .build_extract_value(str_value.into_struct_value(), 1, "str_len_extract")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?);
        }

        if let Operand::Copy(local) | Operand::Move(local) = base {
            if let AirType::Struct(name) = self.local_air_type(*local)?.clone() {
                if !self.local_uses_alloca(*local) {
                    let index = self.struct_field_index(&name, field)?;
                    let struct_value = self.load_local(*local)?.into_struct_value();
                    return Ok(self
                        .builder
                        .build_extract_value(struct_value, index, "field_extract")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))?);
                }
            }
        }

        let (struct_name, struct_ty, base_ptr) = self.struct_pointer_from_operand(base)?;
        let index = self.struct_field_index(&struct_name, field)?;
        let field_ty = self.struct_field_type(&struct_name, field)?;
        let field_ptr = self
            .builder
            .build_struct_gep(struct_ty, base_ptr, index, "field_ptr")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        let llvm_field_ty = air_basic_type_to_llvm(field_ty, self.context)?;
        self.load_value(llvm_field_ty, field_ptr, "field_load")
    }

    pub(crate) fn struct_pointer_from_operand(
        &mut self,
        operand: &Operand,
    ) -> Result<(String, StructType<'static>, PointerValue<'static>), CodegenError> {
        match operand {
            Operand::Copy(local) | Operand::Move(local) => match self.local_air_type(*local)? {
                AirType::Struct(name) => {
                    let ty = self.context.get_struct_type(name).ok_or_else(|| {
                        CodegenError::UnsupportedType(format!("unknown struct {}", name))
                    })?;
                    Ok((name.clone(), ty, self.lookup_local_ptr(*local)?))
                }
                AirType::Ptr(inner) => match inner.as_ref() {
                    AirType::Struct(name) => {
                        let ty = self.context.get_struct_type(name).ok_or_else(|| {
                            CodegenError::UnsupportedType(format!("unknown struct {}", name))
                        })?;
                        Ok((
                            name.clone(),
                            ty,
                            self.load_local(*local)?.into_pointer_value(),
                        ))
                    }
                    other => Err(CodegenError::UnsupportedType(format!(
                        "field access pointer to non-struct {:?}",
                        other
                    ))),
                },
                other => Err(CodegenError::UnsupportedType(format!(
                    "field access on non-struct type {:?}",
                    other
                ))),
            },
            Operand::Const(_) => Err(CodegenError::UnsupportedInstruction(
                "field access on const base is not supported".to_string(),
            )),
        }
    }

    pub(crate) fn struct_field_index(
        &self,
        struct_name: &str,
        field: &str,
    ) -> Result<u32, CodegenError> {
        let index = self
            .program
            .structs
            .iter()
            .find(|s| s.name == struct_name)
            .and_then(|s| s.fields.iter().position(|f| f.name == field))
            .ok_or_else(|| {
                CodegenError::UnsupportedType(format!(
                    "unknown field `{}` on `{}`",
                    field, struct_name
                ))
            })?;

        u32::try_from(index).map_err(|_| {
            CodegenError::UnsupportedType(format!("field index overflow on {}", struct_name))
        })
    }

    pub(crate) fn struct_field_type<'b>(
        &'b self,
        struct_name: &str,
        field: &str,
    ) -> Result<&'b AirType, CodegenError> {
        self.program
            .structs
            .iter()
            .find(|s| s.name == struct_name)
            .and_then(|s| s.fields.iter().find(|f| f.name == field))
            .map(|f| &f.ty)
            .ok_or_else(|| {
                CodegenError::UnsupportedType(format!(
                    "unknown field `{}` on `{}`",
                    field, struct_name
                ))
            })
    }
}

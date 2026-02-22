use crate::CodegenError;
use crate::body::FunctionCodegen;
use crate::types::air_basic_type_to_llvm;
use aelys_air::{AirStmtKind, AirType, Place};
use inkwell::AddressSpace;
use inkwell::types::BasicTypeEnum;
use inkwell::values::PointerValue;

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn generate_stmt(&mut self, stmt: &AirStmtKind) -> Result<(), CodegenError> {
        match stmt {
            AirStmtKind::Assign { place, rvalue } => {
                let expected_ty = self.place_type(place)?;
                let value = self.generate_rvalue(rvalue, Some(&expected_ty))?;
                let ptr = self.place_ptr(place)?;
                self.store_value(ptr, value, &expected_ty)
            }
            AirStmtKind::CallVoid { func, args } => {
                let _ = self.generate_call(func, args, None)?;
                Ok(())
            }
            AirStmtKind::GcAlloc { local, ty, .. } | AirStmtKind::Alloc { local, ty } => {
                let alloc_fn = self.ensure_alloc_function();
                let size = self.air_type_size(ty)? as u64;
                let size_value = self.context.i64_type().const_int(size, false);
                let call = self
                    .builder
                    .build_call(alloc_fn, &[size_value.into()], "alloc_raw")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                let raw_ptr = call
                    .try_as_basic_value()
                    .basic()
                    .ok_or_else(|| CodegenError::LlvmError("__aelys_alloc returned void".to_string()))?
                    .into_pointer_value();

                let local_ty = self.local_air_type(*local)?.clone();
                let target_ty = air_basic_type_to_llvm(&local_ty, self.context)?;
                let target_ptr_ty = match target_ty {
                    BasicTypeEnum::PointerType(ptr) => ptr,
                    _ => {
                        return Err(CodegenError::UnsupportedType(format!(
                            "alloc destination local {} is not a pointer type",
                            local.0
                        )));
                    }
                };

                let casted = self
                    .builder
                    .build_pointer_cast(raw_ptr, target_ptr_ty, "alloc_cast")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                let local_ptr = self.lookup_local_ptr(*local)?;
                self.store_value(local_ptr, casted.into(), &local_ty)
            }
            AirStmtKind::Free(local) => {
                let free_fn = self.ensure_free_function();
                let ptr_value = self.load_local(*local)?;
                let ptr = ptr_value.into_pointer_value();
                let i8_ptr_ty = self.context.ptr_type(AddressSpace::default());
                let casted = self
                    .builder
                    .build_pointer_cast(ptr, i8_ptr_ty, "free_cast")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                self.builder
                    .build_call(free_fn, &[casted.into()], "")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                Ok(())
            }
            AirStmtKind::GcDrop(_)
            | AirStmtKind::ArenaCreate(_)
            | AirStmtKind::ArenaDestroy(_)
            | AirStmtKind::MemoryFence(_) => todo!(),
        }
    }

    fn place_ptr(&mut self, place: &Place) -> Result<PointerValue<'static>, CodegenError> {
        match place {
            Place::Local(local) => self.lookup_local_ptr(*local),
            Place::Field(local, field) => match self.local_air_type(*local)?.clone() {
                AirType::Struct(name) => {
                    let struct_ty = self.context.get_struct_type(&name).ok_or_else(|| {
                        CodegenError::UnsupportedType(format!("unknown struct {}", name))
                    })?;
                    let ptr = self.lookup_local_ptr(*local)?;
                    let index = self.struct_field_index(&name, field)?;
                    self.builder
                        .build_struct_gep(struct_ty, ptr, index, "place_field")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))
                }
                AirType::Ptr(inner) => match inner.as_ref() {
                    AirType::Struct(name) => {
                        let struct_ty = self.context.get_struct_type(name).ok_or_else(|| {
                            CodegenError::UnsupportedType(format!("unknown struct {}", name))
                        })?;
                        let base_ptr = self.load_local(*local)?.into_pointer_value();
                        let index = self.struct_field_index(name, field)?;
                        self.builder
                            .build_struct_gep(struct_ty, base_ptr, index, "place_field")
                            .map_err(|e| CodegenError::LlvmError(e.to_string()))
                    }
                    _ => Err(CodegenError::UnsupportedType(format!(
                        "place field on non-struct pointer local {}",
                        local.0
                    ))),
                },
                _ => Err(CodegenError::UnsupportedType(format!(
                    "place field on non-struct local {}",
                    local.0
                ))),
            },
            Place::Deref(local) => Ok(self.load_local(*local)?.into_pointer_value()),
            Place::Index(_, _) => todo!(),
        }
    }

    fn place_type(&self, place: &Place) -> Result<AirType, CodegenError> {
        match place {
            Place::Local(local) => Ok(self.local_air_type(*local)?.clone()),
            Place::Field(local, field) => {
                let struct_name = match self.local_air_type(*local)? {
                    AirType::Struct(name) => name.as_str(),
                    AirType::Ptr(inner) => match inner.as_ref() {
                        AirType::Struct(name) => name.as_str(),
                        _ => {
                            return Err(CodegenError::UnsupportedType(format!(
                                "field access on non-struct pointer local {}",
                                local.0
                            )));
                        }
                    },
                    _ => {
                        return Err(CodegenError::UnsupportedType(format!(
                            "field access on non-struct local {}",
                            local.0
                        )));
                    }
                };
                Ok(self.struct_field_type(struct_name, field)?.clone())
            }
            Place::Deref(local) => match self.local_air_type(*local)? {
                AirType::Ptr(inner) => Ok((**inner).clone()),
                other => Err(CodegenError::UnsupportedType(format!(
                    "cannot dereference non-pointer place {:?}",
                    other
                ))),
            },
            Place::Index(_, _) => Err(CodegenError::UnsupportedInstruction(
                "index place not implemented".to_string(),
            )),
        }
    }
}

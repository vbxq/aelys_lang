use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::types::air_basic_type_to_llvm;
use aelys_air::{AirStmtKind, AirType, LocalId, Place};
use inkwell::AddressSpace;
use inkwell::types::BasicTypeEnum;
use inkwell::values::{IntValue, PointerValue};

pub(crate) enum ElemBase {
    Slot {
        ptr: PointerValue<'static>,
        arr_ty: AirType,
        len: IntValue<'static>,
    },
    Buffer {
        data: PointerValue<'static>,
        len: IntValue<'static>,
    },
}

/// `a[0..0]` must build a zero-length slice without trapping (f39 puts the trap on the later
pub(crate) enum BoundsCheck {
    Checked,
    Elem0Unchecked,
}

// the data pointer, so the runtime and the inline cow guard recede by this size to reach it.
pub(crate) const RC_HEADER_SIZE: u64 = 16;
const _: () = assert!(RC_HEADER_SIZE == 16);

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn generate_stmt(&mut self, stmt: &AirStmtKind) -> Result<(), CodegenError> {
        match stmt {
            AirStmtKind::Assign { place, rvalue } => {
                let expected_ty = self.place_type(place)?;
                let value = self.generate_rvalue(rvalue, Some(&expected_ty))?;
                match place {
                    Place::Local(local) => self.assign_local(*local, value),
                    _ => {
                        // the single realization point for every indexed store in the language.
                        // post-detach pointer).
                        if let Some((root, inner, through_ptr)) = self.vec_root_of(place)? {
                            self.emit_vec_detach(root, &inner, through_ptr)?;
                        }
                        let ptr = self.place_ptr(place)?;
                        self.store_value(ptr, value)
                    }
                }
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
                    .ok_or_else(|| {
                        CodegenError::LlvmError("__aelys_alloc returned void".to_string())
                    })?
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
                self.assign_local(*local, casted.into())
            }
            AirStmtKind::RcAlloc { local, ty } => self.generate_rc_alloc(*local, ty),
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
            AirStmtKind::GcDrop(_) => Err(self.unsupported_air(
                "AirStmtKind::GcDrop",
                "gc_drop is not implemented for LLVM backend",
            )),
            AirStmtKind::ArenaCreate(_) => Err(self.unsupported_air(
                "AirStmtKind::ArenaCreate",
                "arena_create is not implemented for LLVM backend",
            )),
            AirStmtKind::ArenaDestroy(_) => Err(self.unsupported_air(
                "AirStmtKind::ArenaDestroy",
                "arena_destroy is not implemented for LLVM backend",
            )),
            AirStmtKind::MemoryFence(ordering) => Err(self.unsupported_air(
                "AirStmtKind::MemoryFence",
                format!("memory fence ordering {ordering:?} is not implemented"),
            )),
        }
    }

    fn generate_rc_alloc(&mut self, local: LocalId, data_ty: &AirType) -> Result<(), CodegenError> {
        let alloc_fn = self.ensure_alloc_function();
        let data_size = self.air_type_size(data_ty)? as u64;
        let total = RC_HEADER_SIZE + data_size;
        let size_value = self.context.i64_type().const_int(total, false);
        let call = self
            .builder
            .build_call(alloc_fn, &[size_value.into()], "rc_alloc_raw")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        let base_ptr = call
            .try_as_basic_value()
            .basic()
            .ok_or_else(|| CodegenError::LlvmError("__aelys_alloc returned void".to_string()))?
            .into_pointer_value();

        let i8_ty = self.context.i8_type();
        let i32_ty = self.context.i32_type();
        let store_at = |this: &Self,
                        byte_off: u64,
                        value: inkwell::values::IntValue<'static>|
         -> Result<(), CodegenError> {
            let field_ptr = if byte_off == 0 {
                base_ptr
            } else {
                let idx = this.context.i64_type().const_int(byte_off, false);
                unsafe {
                    this.builder
                        .build_in_bounds_gep(i8_ty, base_ptr, &[idx], "rc_hdr_ptr")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))?
                }
            };
            this.builder
                .build_store(field_ptr, value)
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            Ok(())
        };
        store_at(self, 0, i32_ty.const_int(1, false))?;
        store_at(self, 4, i8_ty.const_int(0, false))?;
        // collect_rc_types sees every RcAlloc, so a miss here is a compiler bug
        let type_id = self
            .program
            .rc_type_table
            .lookup_id(data_ty)
            .ok_or_else(|| {
                CodegenError::LlvmError(format!(
                    "rc_alloc: type {data_ty:?} has no entry in the RC pointer-map table \
                 (collect_rc_types must run before codegen)"
                ))
            })?;
        store_at(self, 8, i32_ty.const_int(type_id as u64, false))?;

        let off16 = self.context.i64_type().const_int(RC_HEADER_SIZE, false);
        let data_ptr = unsafe {
            self.builder
                .build_in_bounds_gep(i8_ty, base_ptr, &[off16], "rc_data_ptr")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?
        };

        let local_ty = self.local_air_type(local)?.clone();
        let target_ty = air_basic_type_to_llvm(&local_ty, self.context)?;
        let target_ptr_ty = match target_ty {
            BasicTypeEnum::PointerType(ptr) => ptr,
            _ => {
                return Err(CodegenError::UnsupportedType(format!(
                    "rc_alloc destination local {} is not a pointer type",
                    local.0
                )));
            }
        };
        let casted = self
            .builder
            .build_pointer_cast(data_ptr, target_ptr_ty, "rc_data_cast")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        self.assign_local(local, casted.into())
    }

    pub(crate) fn place_ptr(
        &mut self,
        place: &Place,
    ) -> Result<PointerValue<'static>, CodegenError> {
        match place {
            Place::Local(local) => self.lookup_local_ptr(*local),
            Place::Global(name) => self.lookup_global_ptr(name),
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
                        self.emit_null_check(base_ptr)?;
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
            Place::Deref(local) => {
                let p = self.load_local(*local)?.into_pointer_value();
                self.emit_null_check(p)?;
                Ok(p)
            }
            Place::Index(local, index_op) => {
                let idx_val = self.generate_operand(index_op)?.into_int_value();
                self.index_ptr(*local, idx_val, BoundsCheck::Checked)
            }
        }
    }

    /// the single array-vs-slice-vs-vec-vs-pointer discriminator. every caller that needs to
    pub(crate) fn elem_base(&mut self, root: LocalId) -> Result<(ElemBase, AirType), CodegenError> {
        let root_ty = self.local_air_type(root)?.clone();
        let (header_ptr, collection) = match &root_ty {
            AirType::Ptr(inner) => {
                let p = self.load_local(root)?.into_pointer_value();
                self.emit_null_check(p)?;
                (p, (**inner).clone())
            }
            other => (self.lookup_local_ptr(root)?, other.clone()),
        };
        match collection {
            AirType::Array(inner, n) => {
                let arr_ty = AirType::Array(inner.clone(), n);
                Ok((
                    ElemBase::Slot {
                        ptr: header_ptr,
                        arr_ty,
                        len: self.context.i64_type().const_int(n, false),
                    },
                    *inner,
                ))
            }
            // through the header pointer, not extracted from a loaded struct, because the
            // pointer form has no loaded struct to extract from
            AirType::Slice(ref inner) | AirType::Vec(ref inner) => {
                let inner = inner.clone();
                let hdr_llvm = air_basic_type_to_llvm(&collection, self.context)?;
                let hdr_struct = match hdr_llvm {
                    BasicTypeEnum::StructType(s) => s,
                    _ => {
                        return Err(CodegenError::UnsupportedType(
                            "slice/vec header is not a struct type".to_string(),
                        ));
                    }
                };
                let data_ptr_slot = self
                    .builder
                    .build_struct_gep(hdr_struct, header_ptr, 0, "buf_ptr_slot")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                let ptr_ty = self.context.ptr_type(AddressSpace::default());
                let data = self
                    .builder
                    .build_load(ptr_ty, data_ptr_slot, "buf_ptr")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?
                    .into_pointer_value();
                let len_slot = self
                    .builder
                    .build_struct_gep(hdr_struct, header_ptr, 1, "buf_len_slot")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                let len = self
                    .builder
                    .build_load(self.context.i64_type(), len_slot, "buf_len")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?
                    .into_int_value();
                Ok((ElemBase::Buffer { data, len }, *inner))
            }
            other => Err(CodegenError::UnsupportedType(format!(
                "cannot index into {:?}",
                other
            ))),
        }
    }

    /// the only caller of `elem_base` for addressing.
    pub(crate) fn index_ptr(
        &mut self,
        root: LocalId,
        idx: IntValue<'static>,
        bounds: BoundsCheck,
    ) -> Result<PointerValue<'static>, CodegenError> {
        Ok(self.index_ptr_and_len(root, idx, bounds)?.0)
    }

// the same gep as index_ptr plus the base length, so a caller needing both cannot drift
    pub(crate) fn index_ptr_and_len(
        &mut self,
        root: LocalId,
        idx: IntValue<'static>,
        bounds: BoundsCheck,
    ) -> Result<(PointerValue<'static>, IntValue<'static>), CodegenError> {
        let (base, elem) = self.elem_base(root)?;
        let base_len = match &base {
            ElemBase::Slot { len, .. } | ElemBase::Buffer { len, .. } => *len,
        };
        let ptr = match base {
            ElemBase::Slot { ptr, arr_ty, len } => {
                if matches!(bounds, BoundsCheck::Checked) {
                    self.emit_bounds_check(idx, len)?;
                }
                let arr_llvm = air_basic_type_to_llvm(&arr_ty, self.context)?;
                let zero = self.context.i64_type().const_zero();
                unsafe {
                    self.builder
                        .build_in_bounds_gep(arr_llvm, ptr, &[zero, idx], "idx_ptr")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))
                }
            }
            ElemBase::Buffer { data, len } => {
                if matches!(bounds, BoundsCheck::Checked) {
                    self.emit_bounds_check(idx, len)?;
                }
                let elem_llvm = air_basic_type_to_llvm(&elem, self.context)?;
                unsafe {
                    self.builder
                        .build_in_bounds_gep(elem_llvm, data, &[idx], "idx_ptr")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))
                }
            }
        }?;
        Ok((ptr, base_len))
    }

    pub(crate) fn collection_len(
        &mut self,
        root: LocalId,
    ) -> Result<IntValue<'static>, CodegenError> {
        let (base, _) = self.elem_base(root)?;
        Ok(match base {
            ElemBase::Slot { len, .. } | ElemBase::Buffer { len, .. } => len,
        })
    }

    /// the only discriminator for the cow detach. it walks the same pointer chain `elem_base`
    /// walks, so a `ptr(vec)` root cannot silently stop matching.
    /// only because a vec inside a struct is e0410 and `& &t` collapses in sema. if either
    /// fence lifts, this must become a depth.
    pub(crate) fn vec_root_of(
        &self,
        place: &Place,
    ) -> Result<Option<(LocalId, AirType, bool)>, CodegenError> {
        let Place::Index(local, _) = place else {
            return Ok(None);
        };
        Ok(match self.local_air_type(*local)? {
            AirType::Vec(inner) => Some((*local, (**inner).clone(), false)),
            AirType::Ptr(outer) => match outer.as_ref() {
                AirType::Vec(inner) => Some((*local, (**inner).clone(), true)),
                _ => None,
            },
            _ => None,
        })
    }

    pub(crate) fn place_type(&self, place: &Place) -> Result<AirType, CodegenError> {
        match place {
            Place::Local(local) => Ok(self.local_air_type(*local)?.clone()),
            Place::Global(name) => Ok(self.lookup_program_global(name)?.ty.clone()),
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
            Place::Index(local, _) => {
                let root = match self.local_air_type(*local)? {
                    AirType::Ptr(inner) => inner.as_ref(),
                    other => other,
                };
                match root {
                    AirType::Array(inner, _) | AirType::Slice(inner) | AirType::Vec(inner) => {
                        Ok((**inner).clone())
                    }
                    other => Err(CodegenError::UnsupportedType(format!(
                        "cannot index into {:?}",
                        other
                    ))),
                }
            }
        }
    }
}


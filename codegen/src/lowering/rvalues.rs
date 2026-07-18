use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::lowering::functions::function_symbol_name;
use crate::types::{air_basic_type_to_llvm, alignment_of, closure_fat_ptr_type};
use aelys_air::layout::{enum_has_data, enum_max_payload_size};
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
            Rvalue::Index { base, index } => self.generate_index(base, index),
            Rvalue::EnumInit {
                enum_name,
                tag,
                payload,
                ..
            } => self.generate_enum_init(enum_name, *tag, payload),
            Rvalue::EnumTag { enum_name, operand } => self.generate_enum_tag(enum_name, operand),
            Rvalue::EnumPayload {
                enum_name,
                tag,
                operand,
                field_index,
            } => self.generate_enum_payload(enum_name, *tag, operand, *field_index),
            Rvalue::ClosureCreate { fn_name, env } => {
                self.generate_closure_create(fn_name, env)
            }
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
            // a Vec indexes through fields 0 and 1 exactly like a Slice
            AirType::Slice(ref inner) | AirType::Vec(ref inner) => {
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
                // UTF-8 char indexing, runtime handles multi-byte scanning
                let str_val = self.generate_operand(base)?.into_struct_value();
                let (str_ptr, str_len) = self.string_parts_from_value(str_val)?;
                let char_at_fn = self.ensure_str_char_at_function();
                self.call_sret_returning_fn(
                    char_at_fn,
                    &[str_ptr.into(), str_len.into(), idx_val.into()],
                    "str_char_at",
                )
            }
            other => Err(CodegenError::UnsupportedType(format!(
                "cannot index into {:?}",
                other
            ))),
        }
    }

    fn generate_enum_init(
        &mut self,
        enum_name: &str,
        tag: u32,
        payload: &[Operand],
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        // Look up the enum def to decide simple vs data
        let enum_def = self.program.enums.iter().find(|e| e.name == enum_name);

        let is_data_enum = enum_def.is_some_and(|d| enum_has_data(d));

        if !is_data_enum || payload.is_empty() {
            // Simple enum or unit variant of a data enum: still need to produce
            // the right type. For data enums, we must produce a { i32, [N x i8] } value.
            if is_data_enum {
                let def = enum_def.expect("invariant: is_data_enum implies the enum def exists");
                let max_payload = enum_max_payload_size(def, &self.program.struct_sizes);
                let enum_struct_name = format!("__aelys_enum_{}", enum_name);
                let enum_ty = self
                    .context
                    .get_struct_type(&enum_struct_name)
                    .ok_or_else(|| {
                        CodegenError::UnsupportedType(format!(
                            "unknown enum struct type: {}",
                            enum_struct_name
                        ))
                    })?;

                let tmp = self
                    .builder
                    .build_alloca(enum_ty, "enum_tmp")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                self.align_alloca(tmp, enum_ty.into())?;

                // Store the tag
                let tag_ptr = self
                    .builder
                    .build_struct_gep(enum_ty, tmp, 0, "enum_tag_ptr")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                let tag_val = self.context.i32_type().const_int(tag as u64, false);
                self.store_value(tag_ptr, tag_val.into())?;

                // Zero-init the payload area
                if max_payload > 0 {
                    let payload_ptr = self
                        .builder
                        .build_struct_gep(enum_ty, tmp, 1, "enum_payload_ptr")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                    let payload_arr_ty = self.context.i8_type().array_type(max_payload);
                    let zero = payload_arr_ty.const_zero();
                    self.store_value(payload_ptr, zero.into())?;
                }

                self.load_value(enum_ty.into(), tmp, "enum_value")
            } else {
                // Pure simple enum: just an i32 tag
                Ok(self.context.i32_type().const_int(tag as u64, false).into())
            }
        } else {
            // Data variant construction: build { i32 tag, [N x i8] payload }
            let def = enum_def.expect("invariant: is_data_enum implies the enum def exists");
            let max_payload = enum_max_payload_size(def, &self.program.struct_sizes);
            let enum_struct_name = format!("__aelys_enum_{}", enum_name);
            let enum_ty = self
                .context
                .get_struct_type(&enum_struct_name)
                .ok_or_else(|| {
                    CodegenError::UnsupportedType(format!(
                        "unknown enum struct type: {}",
                        enum_struct_name
                    ))
                })?;

            let tmp = self
                .builder
                .build_alloca(enum_ty, "enum_tmp")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            self.align_alloca(tmp, enum_ty.into())?;

            // Store the tag at index 0
            let tag_ptr = self
                .builder
                .build_struct_gep(enum_ty, tmp, 0, "enum_tag_ptr")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            let tag_val = self.context.i32_type().const_int(tag as u64, false);
            self.store_value(tag_ptr, tag_val.into())?;

            // Zero-init the full payload area first so trailing bytes are clean
            if max_payload > 0 {
                let payload_ptr = self
                    .builder
                    .build_struct_gep(enum_ty, tmp, 1, "enum_payload_ptr")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                let payload_arr_ty = self.context.i8_type().array_type(max_payload);
                let zero = payload_arr_ty.const_zero();
                self.store_value(payload_ptr, zero.into())?;
            }

            // Store each payload field at the right offset within the byte array
            let payload_base_ptr = self
                .builder
                .build_struct_gep(enum_ty, tmp, 1, "enum_payload_base")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

            // Find the variant definition to get the field types
            let variant_def = def.variants.iter().find(|v| v.tag == tag).ok_or_else(|| {
                CodegenError::LlvmError(format!(
                    "unknown variant tag {} for enum {}",
                    tag, enum_name
                ))
            })?;

            let mut byte_offset: u32 = 0;
            for (i, (operand, field_air_ty)) in
                payload.iter().zip(variant_def.payload.iter()).enumerate()
            {
                let field_llvm_ty = air_basic_type_to_llvm(field_air_ty, self.context)?;
                let field_layout = aelys_air::layout::resolved_layout(field_air_ty, &self.program.struct_sizes);

                // Align the offset
                byte_offset = (byte_offset + field_layout.align - 1) & !(field_layout.align - 1);

                // GEP into the byte array at the current offset, then bitcast to field type pointer
                let offset_val = self.context.i32_type().const_int(byte_offset as u64, false);
                let field_ptr = unsafe {
                    self.builder.build_in_bounds_gep(
                        self.context.i8_type(),
                        payload_base_ptr,
                        &[offset_val],
                        &format!("enum_field_{}_ptr", i),
                    )
                }
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

                let value = self.generate_operand(operand)?;

                // The payload byte array sits at struct offset 4 (after the i32
                // tag) inside the non-packed struct { i32, [N x i8] }.  The struct
                // alignment is 4, so the payload base is 4-byte aligned.  We must
                // not claim a higher alignment than the address actually has.
                let field_align = alignment_of(field_llvm_ty).min(4);
                let store = self
                    .builder
                    .build_store(field_ptr, value)
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                store
                    .set_alignment(field_align)
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

                byte_offset += field_layout.size;
            }

            self.load_value(enum_ty.into(), tmp, "enum_value")
        }
    }

    fn generate_enum_tag(
        &mut self,
        enum_name: &str,
        operand: &Operand,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let enum_def = self.program.enums.iter().find(|e| e.name == enum_name);

        let is_data_enum = enum_def.is_some_and(|d| enum_has_data(d));

        if is_data_enum {
            // Data enum: { i32 tag, [N x i8] payload } -- extract field 0
            let enum_struct_name = format!("__aelys_enum_{}", enum_name);
            let enum_ty = self
                .context
                .get_struct_type(&enum_struct_name)
                .ok_or_else(|| {
                    CodegenError::UnsupportedType(format!(
                        "unknown enum struct type: {}",
                        enum_struct_name
                    ))
                })?;

            // The operand is a struct value. We need it on the stack to GEP into it.
            let val = self.generate_operand(operand)?;
            let tmp = self
                .builder
                .build_alloca(enum_ty, "match_enum_tmp")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            self.align_alloca(tmp, enum_ty.into())?;
            self.store_value(tmp, val)?;

            let tag_ptr = self
                .builder
                .build_struct_gep(enum_ty, tmp, 0, "match_tag_ptr")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            self.load_value(self.context.i32_type().into(), tag_ptr, "match_tag")
        } else {
            // Simple enum: the value IS the i32 tag
            self.generate_operand(operand)
        }
    }

    fn generate_enum_payload(
        &mut self,
        enum_name: &str,
        tag: u32,
        operand: &Operand,
        field_index: u32,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let def = self
            .program
            .enums
            .iter()
            .find(|e| e.name == enum_name)
            .ok_or_else(|| CodegenError::UnsupportedType(format!("unknown enum: {}", enum_name)))?;

        let variant_def = def.variants.iter().find(|v| v.tag == tag).ok_or_else(|| {
            CodegenError::LlvmError(format!(
                "unknown variant tag {} for enum {}",
                tag, enum_name
            ))
        })?;

        if field_index as usize >= variant_def.payload.len() {
            return Err(CodegenError::LlvmError(format!(
                "field index {} out of range for variant (has {} fields)",
                field_index,
                variant_def.payload.len()
            )));
        }

        let field_air_ty = &variant_def.payload[field_index as usize];
        let field_llvm_ty = air_basic_type_to_llvm(field_air_ty, self.context)?;

        let enum_struct_name = format!("__aelys_enum_{}", enum_name);
        let enum_ty = self
            .context
            .get_struct_type(&enum_struct_name)
            .ok_or_else(|| {
                CodegenError::UnsupportedType(format!(
                    "unknown enum struct type: {}",
                    enum_struct_name
                ))
            })?;

        // Store the operand on the stack so we can GEP into it
        let val = self.generate_operand(operand)?;
        let tmp = self
            .builder
            .build_alloca(enum_ty, "match_payload_tmp")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        self.align_alloca(tmp, enum_ty.into())?;
        self.store_value(tmp, val)?;

        // GEP to the payload byte array (field 1 of the enum struct)
        let payload_base_ptr = self
            .builder
            .build_struct_gep(enum_ty, tmp, 1, "match_payload_base")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        // Calculate the byte offset of the requested field, matching the layout used in EnumInit
        let mut byte_offset: u32 = 0;
        for i in 0..=field_index {
            let ty = &variant_def.payload[i as usize];
            let layout = aelys_air::layout::resolved_layout(ty, &self.program.struct_sizes);
            // Align before this field
            byte_offset = (byte_offset + layout.align - 1) & !(layout.align - 1);
            if i < field_index {
                byte_offset += layout.size;
            }
        }

        // GEP into the byte array at the computed offset
        let offset_val = self.context.i32_type().const_int(byte_offset as u64, false);
        let field_ptr = unsafe {
            self.builder.build_in_bounds_gep(
                self.context.i8_type(),
                payload_base_ptr,
                &[offset_val],
                &format!("match_field_{}_ptr", field_index),
            )
        }
        .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        // The payload byte array sits at struct offset 4 — see comment in
        // generate_enum_init for why alignment is capped at 4.
        let field_align = alignment_of(field_llvm_ty).min(4);
        let load = self
            .builder
            .build_load(
                field_llvm_ty,
                field_ptr,
                &format!("match_field_{}", field_index),
            )
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        load.as_instruction_value()
            .unwrap()
            .set_alignment(field_align)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        Ok(load)
    }

    // Builds a fat pointer { fn_ptr, env_ptr } for a closure or a named
    // function used as a value. env is either a heap-allocated env struct
    // pointer (capturing closure) or null (non-capturing lambda, named fn).
    fn generate_closure_create(
        &mut self,
        fn_name: &str,
        env: &Operand,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let symbol_name = self
            .program
            .functions
            .iter()
            .find(|f| f.name == *fn_name)
            .map(function_symbol_name)
            .unwrap_or_else(|| fn_name.to_string());
        let func = self.module.get_function(&symbol_name).ok_or_else(|| {
            CodegenError::LlvmError(format!("closure_create: unknown function '{}'", fn_name))
        })?;
        let fn_ptr = func.as_global_value().as_pointer_value();

        // generate the env operand (pointer or null)
        let env_ptr = self.generate_operand(env)?;

        // Build { ptr fn_ptr, ptr env_ptr } struct
        let fat_ty = closure_fat_ptr_type(self.context);
        let mut fat = fat_ty.get_undef();
        fat = self
            .builder
            .build_insert_value(fat, fn_ptr, 0, "closure_fn")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?
            .into_struct_value();
        fat = self
            .builder
            .build_insert_value(fat, env_ptr, 1, "closure_env")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?
            .into_struct_value();
        Ok(fat.into())
    }
}

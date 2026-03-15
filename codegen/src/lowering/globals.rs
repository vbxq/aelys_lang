use crate::CodegenContext;
use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::lowering::functions::function_symbol_name;
use crate::types::{aelys_string_type, air_basic_type_to_llvm, closure_fat_ptr_type};
use aelys_air::{
    AirConst, AirEnumDef, AirGlobal, AirProgram, AirType, Operand,
    layout::{enum_has_data, enum_max_payload_size, resolved_layout},
};
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::values::{BasicValueEnum, IntValue, PointerValue};

pub(crate) const GLOBAL_GET_PREFIX: &str = "__aelys_global_get_";
pub(crate) const GLOBAL_SET_PREFIX: &str = "__aelys_global_set_";
const GLOBAL_STORAGE_PREFIX: &str = "__aelys_global_";

pub(crate) fn global_storage_name(name: &str) -> String {
    format!("{GLOBAL_STORAGE_PREFIX}{name}")
}

impl CodegenContext {
    pub(crate) fn declare_globals(&self, program: &AirProgram) -> Result<(), CodegenError> {
        for global in &program.globals {
            let llvm_ty = air_basic_type_to_llvm(&global.ty, self.context)?;
            let symbol = global_storage_name(&global.name);
            let global_value = if let Some(existing) = self.module.get_global(&symbol) {
                existing
            } else {
                self.module.add_global(llvm_ty, None, &symbol)
            };
            global_value.set_linkage(Linkage::Internal);

            let init = self.global_initializer(global, program)?;
            global_value.set_initializer(&init);
        }

        Ok(())
    }

    fn global_initializer(
        &self,
        global: &AirGlobal,
        program: &AirProgram,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let Some(init) = global.init.as_ref() else {
            return Err(CodegenError::UnsupportedInstruction(format!(
                "global '{}' requires a compile-time constant initializer",
                global.name
            )));
        };

        match init {
            AirConst::IntLiteral(value) => self.int_initializer(&global.ty, *value, program),
            AirConst::Int(value, _) => self.int_initializer(&global.ty, *value, program),
            AirConst::Float(value, _) => self.float_initializer(&global.ty, *value),
            AirConst::Bool(value) => {
                if !matches!(global.ty, AirType::Bool) {
                    return Err(CodegenError::UnsupportedType(format!(
                        "global '{}' has bool initializer but non-bool type {:?}",
                        global.name, global.ty
                    )));
                }
                Ok(self
                    .context
                    .bool_type()
                    .const_int(u64::from(*value), false)
                    .into())
            }
            AirConst::Str(text) => self.string_initializer(&global.name, &global.ty, text),
            AirConst::Null => match global.ty {
                AirType::Ptr(_) => Ok(self
                    .context
                    .ptr_type(AddressSpace::default())
                    .const_null()
                    .into()),
                _ => Err(CodegenError::UnsupportedType(format!(
                    "global '{}' uses null initializer with non-pointer type {:?}",
                    global.name, global.ty
                ))),
            },
            AirConst::FnRef(name) => self.fnref_initializer(&global.name, &global.ty, name, program),
            AirConst::Enum {
                enum_name,
                tag,
                payload,
            } => self.enum_initializer(&global.name, &global.ty, enum_name, *tag, payload, program),
            AirConst::ZeroInit(ty) if *ty == global.ty => {
                Ok(air_basic_type_to_llvm(ty, self.context)?.const_zero())
            }
            AirConst::ZeroInit(ty) => Err(CodegenError::UnsupportedType(format!(
                "global '{}' has mismatched zeroinit type {:?} for {:?}",
                global.name, ty, global.ty
            ))),
            AirConst::Array(elems) => self.array_initializer(&global.name, &global.ty, elems, program),
            other => Err(CodegenError::UnsupportedInstruction(format!(
                "global '{}' has unsupported initializer kind {}",
                global.name,
                crate::lowering::operands::constant_kind_name(other)
            ))),
        }
    }

    fn array_initializer(
        &self,
        global_name: &str,
        ty: &AirType,
        elems: &[AirConst],
        program: &AirProgram,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let AirType::Array(elem_ty, _) = ty else {
            return Err(CodegenError::UnsupportedType(format!(
                "global '{}' has Array initializer but non-array type {:?}",
                global_name, ty
            )));
        };
        let elem_llvm_ty = air_basic_type_to_llvm(elem_ty, self.context)?;
        // Build a temporary AirGlobal for each element so we can reuse
        // the existing scalar initializer paths.
        let elem_consts: Result<Vec<BasicValueEnum<'static>>, _> = elems
            .iter()
            .map(|c| {
                let elem_global = AirGlobal {
                    name: global_name.to_string(),
                    ty: (**elem_ty).clone(),
                    init: Some(c.clone()),
                    gc_mode: aelys_air::GcMode::Manual,
                    span: None,
                };
                self.global_initializer(&elem_global, program)
            })
            .collect();
        let elem_values = elem_consts?;

        // Build the LLVM const array for the element type.
        let const_arr: BasicValueEnum<'static> = match elem_llvm_ty {
            inkwell::types::BasicTypeEnum::IntType(t) => {
                let vals: Vec<_> = elem_values
                    .iter()
                    .map(|v| v.into_int_value())
                    .collect();
                t.const_array(&vals).into()
            }
            inkwell::types::BasicTypeEnum::FloatType(t) => {
                let vals: Vec<_> = elem_values
                    .iter()
                    .map(|v| v.into_float_value())
                    .collect();
                t.const_array(&vals).into()
            }
            inkwell::types::BasicTypeEnum::PointerType(t) => {
                let vals: Vec<_> = elem_values
                    .iter()
                    .map(|v| v.into_pointer_value())
                    .collect();
                t.const_array(&vals).into()
            }
            inkwell::types::BasicTypeEnum::StructType(t) => {
                let vals: Vec<_> = elem_values
                    .iter()
                    .map(|v| v.into_struct_value())
                    .collect();
                t.const_array(&vals).into()
            }
            inkwell::types::BasicTypeEnum::ArrayType(t) => {
                let vals: Vec<_> = elem_values
                    .iter()
                    .map(|v| v.into_array_value())
                    .collect();
                t.const_array(&vals).into()
            }
            other => {
                return Err(CodegenError::UnsupportedType(format!(
                    "global array '{}' has unsupported element type {:?}",
                    global_name, other
                )));
            }
        };
        Ok(const_arr)
    }

    fn int_initializer(
        &self,
        ty: &AirType,
        value: i64,
        program: &AirProgram,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let const_value = match ty {
            AirType::I8 => self.context.i8_type().const_int(value as u64, true).into(),
            AirType::I16 => self.context.i16_type().const_int(value as u64, true).into(),
            AirType::I32 => self.context.i32_type().const_int(value as u64, true).into(),
            AirType::I64 => self.context.i64_type().const_int(value as u64, true).into(),
            AirType::U8 => self.context.i8_type().const_int(value as u64, false).into(),
            AirType::U16 => self.context.i16_type().const_int(value as u64, false).into(),
            AirType::U32 => self.context.i32_type().const_int(value as u64, false).into(),
            AirType::U64 => self.context.i64_type().const_int(value as u64, false).into(),
            AirType::Enum(name) => self.enum_int_initializer(name, value, program)?,
            other => {
                return Err(CodegenError::UnsupportedType(format!(
                    "integer global initializer is not supported for {:?}",
                    other
                )));
            }
        };
        Ok(const_value)
    }

    fn enum_int_initializer(
        &self,
        name: &str,
        value: i64,
        program: &AirProgram,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let enum_struct_name = format!("__aelys_enum_{}", name);
        if self.context.get_struct_type(&enum_struct_name).is_none() {
            return Ok(self.context.i32_type().const_int(value as u64, false).into());
        }

        let enum_def = program
            .enums
            .iter()
            .find(|def| def.name == name)
            .ok_or_else(|| CodegenError::UnsupportedType(format!("unknown enum type {:?}", name)))?;
        if !enum_has_data(enum_def) {
            return Ok(self.context.i32_type().const_int(value as u64, false).into());
        }

        let tag = u32::try_from(value).map_err(|_| {
            CodegenError::UnsupportedType(format!(
                "enum global initializer tag {value} is out of range for {name}"
            ))
        })?;
        let variant = enum_def
            .variants
            .iter()
            .find(|variant| variant.tag == tag)
            .ok_or_else(|| {
                CodegenError::UnsupportedType(format!(
                    "enum global initializer tag {tag} does not exist on {name}"
                ))
            })?;
        if !variant.payload.is_empty() {
            return Err(CodegenError::UnsupportedInstruction(format!(
                "global enum '{}' needs payload data for variant '{}'",
                name, variant.name
            )));
        }

        // Unit variants in data enums still use aggregate storage, so keep the
        // payload byte array zeroed while materializing the tag as a constant.
        let enum_ty = self
            .context
            .get_struct_type(&enum_struct_name)
            .ok_or_else(|| {
                CodegenError::UnsupportedType(format!("unknown enum struct type: {}", enum_struct_name))
            })?;
        let payload_len = enum_max_payload_size(enum_def, &program.struct_sizes);
        let payload = self
            .context
            .i8_type()
            .array_type(payload_len)
            .const_zero();
        Ok(enum_ty
            .const_named_struct(&[
                self.context.i32_type().const_int(tag as u64, false).into(),
                payload.into(),
            ])
            .into())
    }

    fn enum_initializer(
        &self,
        global_name: &str,
        ty: &AirType,
        enum_name: &str,
        tag: u32,
        payload: &[AirConst],
        program: &AirProgram,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let AirType::Enum(global_enum_name) = ty else {
            return Err(CodegenError::UnsupportedType(format!(
                "global '{}' uses enum initializer with non-enum type {:?}",
                global_name, ty
            )));
        };
        if global_enum_name != enum_name {
            return Err(CodegenError::UnsupportedType(format!(
                "global '{}' enum initializer name '{}' does not match declared type '{}'",
                global_name, enum_name, global_enum_name
            )));
        }

        let enum_def = program
            .enums
            .iter()
            .find(|def| def.name == enum_name)
            .ok_or_else(|| CodegenError::UnsupportedType(format!("unknown enum type {:?}", enum_name)))?;
        if !enum_has_data(enum_def) {
            if !payload.is_empty() {
                return Err(CodegenError::UnsupportedInstruction(format!(
                    "simple enum '{}' cannot carry payload data in global '{}'",
                    enum_name, global_name
                )));
            }
            return Ok(self.context.i32_type().const_int(tag as u64, false).into());
        }

        let variant = enum_def
            .variants
            .iter()
            .find(|variant| variant.tag == tag)
            .ok_or_else(|| {
                CodegenError::UnsupportedType(format!(
                    "enum global initializer tag {tag} does not exist on {enum_name}"
                ))
            })?;
        if payload.len() != variant.payload.len() {
            return Err(CodegenError::UnsupportedInstruction(format!(
                "global '{}' enum variant '{}' expected {} payload fields, found {}",
                global_name,
                variant.name,
                variant.payload.len(),
                payload.len()
            )));
        }

        let enum_struct_name = format!("__aelys_enum_{}", enum_name);
        let enum_ty = self
            .context
            .get_struct_type(&enum_struct_name)
            .ok_or_else(|| {
                CodegenError::UnsupportedType(format!("unknown enum struct type: {}", enum_struct_name))
            })?;
        let payload_len = enum_max_payload_size(enum_def, &program.struct_sizes);
        let payload_bytes =
            self.enum_payload_initializer_bytes(global_name, enum_def, variant, payload, program)?;
        let payload = self.context.i8_type().const_array(&payload_bytes);
        debug_assert_eq!(payload_len as usize, payload_bytes.len());
        Ok(enum_ty
            .const_named_struct(&[
                self.context.i32_type().const_int(tag as u64, false).into(),
                payload.into(),
            ])
            .into())
    }

    fn float_initializer(
        &self,
        ty: &AirType,
        value: f64,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let const_value = match ty {
            AirType::F32 => self.context.f32_type().const_float(value).into(),
            AirType::F64 => self.context.f64_type().const_float(value).into(),
            other => {
                return Err(CodegenError::UnsupportedType(format!(
                    "float global initializer is not supported for {:?}",
                    other
                )));
            }
        };
        Ok(const_value)
    }

    fn string_initializer(
        &self,
        name: &str,
        ty: &AirType,
        text: &str,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        if !matches!(ty, AirType::Str) {
            return Err(CodegenError::UnsupportedType(format!(
                "string global initializer is not supported for {:?}",
                ty
            )));
        }

        let bytes = text.as_bytes();
        let array_len = u32::try_from(bytes.len() + 1).map_err(|_| {
            CodegenError::UnsupportedType(format!("string global '{}' is too large", name))
        })?;
        let array_ty = self.context.i8_type().array_type(array_len);
        let storage_name = format!("{}{}_bytes", GLOBAL_STORAGE_PREFIX, name);
        let backing = if let Some(existing) = self.module.get_global(&storage_name) {
            existing
        } else {
            let global_value = self.module.add_global(array_ty, None, &storage_name);
            global_value.set_linkage(Linkage::Private);
            global_value.set_constant(true);
            let mut nul_terminated = bytes.to_vec();
            nul_terminated.push(0);
            let byte_values: Vec<_> = nul_terminated
                .iter()
                .map(|byte| self.context.i8_type().const_int(*byte as u64, false))
                .collect();
            global_value.set_initializer(&self.context.i8_type().const_array(&byte_values));
            global_value
        };

        let zero = self.context.i64_type().const_zero();
        let ptr = unsafe {
            backing
                .as_pointer_value()
                .const_in_bounds_gep(array_ty, &[zero, zero])
        };
        let len = self.context.i64_type().const_int(bytes.len() as u64, false);
        Ok(aelys_string_type(self.context)
            .const_named_struct(&[ptr.into(), len.into()])
            .into())
    }

    fn fnref_initializer(
        &self,
        global_name: &str,
        ty: &AirType,
        function_name: &str,
        program: &AirProgram,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let conv = match ty {
            AirType::FnPtr { conv, .. } => *conv,
            _ => {
                return Err(CodegenError::UnsupportedType(format!(
                    "global '{}' uses fnref initializer with non-fn type {:?}",
                    global_name, ty
                )));
            }
        };

        let symbol_name = program
            .functions
            .iter()
            .find(|function| function.name == function_name)
            .map(function_symbol_name)
            .unwrap_or_else(|| function_name.to_string());
        // Globals are emitted before bodies, so they need the declared LLVM symbol.
        let func = self.module.get_function(&symbol_name).ok_or_else(|| {
            CodegenError::LlvmError(format!(
                "global '{}' references unknown function '{}'",
                global_name, function_name
            ))
        })?;
        let fn_ptr = func.as_global_value().as_pointer_value();

        if matches!(conv, aelys_air::CallingConv::Aelys) {
            // Aelys-convention function values are fat pointers { fn_ptr, env_ptr }.
            // Named functions have no captures, so env_ptr is null.
            let null_env = self
                .context
                .ptr_type(AddressSpace::default())
                .const_null();
            let fat = closure_fat_ptr_type(self.context)
                .const_named_struct(&[fn_ptr.into(), null_env.into()]);
            Ok(fat.into())
        } else {
            Ok(fn_ptr.into())
        }
    }

    fn enum_payload_initializer_bytes(
        &self,
        global_name: &str,
        enum_def: &AirEnumDef,
        variant: &aelys_air::AirEnumVariant,
        payload: &[AirConst],
        program: &AirProgram,
    ) -> Result<Vec<IntValue<'static>>, CodegenError> {
        let payload_len = enum_max_payload_size(enum_def, &program.struct_sizes);
        let mut bytes = vec![self.context.i8_type().const_zero(); payload_len as usize];
        let mut byte_offset = 0u32;

        for (index, (field_ty, field_const)) in variant.payload.iter().zip(payload.iter()).enumerate() {
            let field_layout = resolved_layout(field_ty, &program.struct_sizes);
            byte_offset = align_to(byte_offset, field_layout.align);

            // Constant globals still store enum payloads in the raw byte array layout.
            // Pack fields with the same AIR-computed offsets as runtime EnumInit.
            let field_bytes = self.const_bytes(
                &format!("{global_name}_{}_{}", variant.name, index),
                field_ty,
                field_const,
                program,
            )?;
            if field_bytes.len() != field_layout.size as usize {
                return Err(CodegenError::UnsupportedInstruction(format!(
                    "global '{}' field {} for enum '{}' serialized to {} bytes, expected {}",
                    global_name,
                    index,
                    enum_def.name,
                    field_bytes.len(),
                    field_layout.size
                )));
            }
            let start = byte_offset as usize;
            let end = start + field_bytes.len();
            bytes[start..end].clone_from_slice(&field_bytes);
            byte_offset += field_layout.size;
        }

        Ok(bytes)
    }

    fn const_bytes(
        &self,
        name: &str,
        ty: &AirType,
        constant: &AirConst,
        program: &AirProgram,
    ) -> Result<Vec<IntValue<'static>>, CodegenError> {
        match (ty, constant) {
            (AirType::I8 | AirType::U8, AirConst::Int(value, _))
            | (AirType::I8 | AirType::U8, AirConst::IntLiteral(value)) => Ok(vec![
                self.context.i8_type().const_int(*value as u64, false),
            ]),
            (AirType::Bool, AirConst::Bool(value)) => {
                Ok(vec![self.context.i8_type().const_int(u64::from(*value), false)])
            }
            (
                AirType::I16
                | AirType::I32
                | AirType::I64
                | AirType::U16
                | AirType::U32
                | AirType::U64,
                AirConst::Int(value, _) | AirConst::IntLiteral(value),
            ) => self.integer_bytes(ty, *value),
            (AirType::F32, AirConst::Float(value, _)) => Ok(f32::to_le_bytes(*value as f32)
                .into_iter()
                .map(|byte| self.context.i8_type().const_int(byte as u64, false))
                .collect()),
            (AirType::F64, AirConst::Float(value, _)) => Ok(f64::to_le_bytes(*value)
                .into_iter()
                .map(|byte| self.context.i8_type().const_int(byte as u64, false))
                .collect()),
            (AirType::Ptr(_), AirConst::Null) => Ok(vec![self.context.i8_type().const_zero(); 8]),
            (AirType::Enum(enum_name), AirConst::Int(value, _) | AirConst::IntLiteral(value)) => {
                let tag = u32::try_from(*value).map_err(|_| {
                    CodegenError::UnsupportedType(format!(
                        "enum initializer tag {value} is out of range for {enum_name}"
                    ))
                })?;
                self.enum_value_bytes(name, enum_name, tag, &[], program)
            }
            (
                AirType::Enum(enum_name),
                AirConst::Enum {
                    enum_name: const_enum_name,
                    tag,
                    payload,
                },
            ) => {
                if enum_name != const_enum_name {
                    return Err(CodegenError::UnsupportedType(format!(
                        "nested enum constant '{}' does not match expected '{}'",
                        const_enum_name, enum_name
                    )));
                }
                self.enum_value_bytes(name, enum_name, *tag, payload, program)
            }
            _ => Err(CodegenError::UnsupportedInstruction(format!(
                "global '{}' cannot serialize {} as {:?}",
                name,
                crate::lowering::operands::constant_kind_name(constant),
                ty
            ))),
        }
    }

    fn enum_value_bytes(
        &self,
        name: &str,
        enum_name: &str,
        tag: u32,
        payload: &[AirConst],
        program: &AirProgram,
    ) -> Result<Vec<IntValue<'static>>, CodegenError> {
        let enum_def = program
            .enums
            .iter()
            .find(|def| def.name == enum_name)
            .ok_or_else(|| CodegenError::UnsupportedType(format!("unknown enum type {:?}", enum_name)))?;
        if !enum_has_data(enum_def) {
            return self.integer_bytes(&AirType::I32, tag as i64);
        }

        let layout = resolved_layout(&AirType::Enum(enum_name.to_string()), &program.struct_sizes);
        let payload_align = enum_payload_align(enum_def, &program.struct_sizes);
        let payload_offset = align_to(4, payload_align);
        let variant = enum_def
            .variants
            .iter()
            .find(|variant| variant.tag == tag)
            .ok_or_else(|| {
                CodegenError::UnsupportedType(format!(
                    "enum initializer tag {tag} does not exist on {enum_name}"
                ))
            })?;
        if payload.len() != variant.payload.len() {
            return Err(CodegenError::UnsupportedInstruction(format!(
                "enum '{}' expected {} payload fields for tag {}, found {}",
                enum_name,
                variant.payload.len(),
                tag,
                payload.len()
            )));
        }

        let mut bytes = vec![self.context.i8_type().const_zero(); layout.size as usize];
        let tag_bytes = self.integer_bytes(&AirType::I32, tag as i64)?;
        bytes[..4].clone_from_slice(&tag_bytes);
        let payload_bytes = self.enum_payload_initializer_bytes(name, enum_def, variant, payload, program)?;
        let start = payload_offset as usize;
        let end = start + payload_bytes.len();
        bytes[start..end].clone_from_slice(&payload_bytes);
        Ok(bytes)
    }

    fn integer_bytes(
        &self,
        ty: &AirType,
        value: i64,
    ) -> Result<Vec<IntValue<'static>>, CodegenError> {
        let bytes = match ty {
            AirType::I8 => vec![(value as i8) as u8],
            AirType::U8 => vec![value as u8],
            AirType::I16 => (value as i16).to_le_bytes().to_vec(),
            AirType::U16 => (value as u16).to_le_bytes().to_vec(),
            AirType::I32 => (value as i32).to_le_bytes().to_vec(),
            AirType::U32 => (value as u32).to_le_bytes().to_vec(),
            AirType::I64 => value.to_le_bytes().to_vec(),
            AirType::U64 => (value as u64).to_le_bytes().to_vec(),
            other => {
                return Err(CodegenError::UnsupportedType(format!(
                    "integer byte serialization is not supported for {:?}",
                    other
                )));
            }
        };
        Ok(bytes
            .into_iter()
            .map(|byte| self.context.i8_type().const_int(byte as u64, false))
            .collect())
    }
}

fn align_to(offset: u32, align: u32) -> u32 {
    (offset + align - 1) & !(align - 1)
}

fn enum_payload_align(
    def: &AirEnumDef,
    sizes: &std::collections::HashMap<String, aelys_air::layout::TypeLayout>,
) -> u32 {
    def.variants
        .iter()
        .flat_map(|variant| variant.payload.iter())
        .map(|ty| resolved_layout(ty, sizes).align)
        .max()
        .unwrap_or(1)
}

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn generate_global_get(
        &mut self,
        name: &str,
        args: &[Operand],
    ) -> Result<Option<BasicValueEnum<'static>>, CodegenError> {
        if !args.is_empty() {
            return Err(CodegenError::UnsupportedInstruction(format!(
                "global getter '{}' does not take arguments",
                name
            )));
        }

        let global = self.lookup_program_global(name)?;
        let ptr = self.lookup_global_ptr(name)?;
        let llvm_ty = air_basic_type_to_llvm(&global.ty, self.context)?;
        Ok(Some(self.load_value(llvm_ty, ptr, "global_load")?))
    }

    pub(crate) fn generate_global_set(
        &mut self,
        name: &str,
        args: &[Operand],
    ) -> Result<Option<BasicValueEnum<'static>>, CodegenError> {
        if args.len() != 1 {
            return Err(CodegenError::UnsupportedInstruction(format!(
                "global setter '{}' expects exactly one argument",
                name
            )));
        }

        let ptr = self.lookup_global_ptr(name)?;
        let value = self.generate_operand(&args[0])?;
        self.store_value(ptr, value)?;
        Ok(None)
    }

    fn lookup_program_global(&self, name: &str) -> Result<&AirGlobal, CodegenError> {
        self.program
            .globals
            .iter()
            .find(|global| global.name == name)
            .ok_or_else(|| CodegenError::UnsupportedInstruction(format!("unknown global '{}'", name)))
    }

    fn lookup_global_ptr(&self, name: &str) -> Result<PointerValue<'static>, CodegenError> {
        self.module
            .get_global(&global_storage_name(name))
            .map(|global| global.as_pointer_value())
            .ok_or_else(|| CodegenError::LlvmError(format!("missing LLVM global '{}'", name)))
    }
}

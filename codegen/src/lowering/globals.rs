use crate::CodegenContext;
use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::lowering::functions::function_symbol_name;
use crate::types::{aelys_string_type, air_basic_type_to_llvm};
use aelys_air::{
    AirConst, AirGlobal, AirProgram, AirType, Operand,
    layout::{enum_has_data, enum_max_payload_size},
};
use inkwell::AddressSpace;
use inkwell::module::Linkage;
use inkwell::values::{BasicValueEnum, PointerValue};

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
            AirConst::ZeroInit(ty) if *ty == global.ty => {
                Ok(air_basic_type_to_llvm(ty, self.context)?.const_zero())
            }
            AirConst::ZeroInit(ty) => Err(CodegenError::UnsupportedType(format!(
                "global '{}' has mismatched zeroinit type {:?} for {:?}",
                global.name, ty, global.ty
            ))),
            other => Err(CodegenError::UnsupportedInstruction(format!(
                "global '{}' has unsupported initializer kind {}",
                global.name,
                crate::lowering::operands::constant_kind_name(other)
            ))),
        }
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
        if !matches!(ty, AirType::FnPtr { .. }) {
            return Err(CodegenError::UnsupportedType(format!(
                "global '{}' uses fnref initializer with non-fn type {:?}",
                global_name, ty
            )));
        }

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
        Ok(func.as_global_value().as_pointer_value().into())
    }
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

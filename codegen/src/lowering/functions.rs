use crate::CodegenContext;
use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::types::{air_basic_type_to_llvm, air_type_to_llvm};
use crate::{is_reserved_bootstrap_builtin, reserved_bootstrap_builtin_message};
use aelys_air::{
    AirFunction, AirProgram, AirType, CallingConv as AirCallingConv, FunctionAttribs, InlineHint,
    layout::enum_has_data, symbols::NATIVE_ENTRY_SYMBOL,
};
use inkwell::AddressSpace;
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::llvm_sys::LLVMCallConv;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, FunctionType};
use inkwell::values::FunctionValue;
use std::collections::HashMap;

// Calling convention for Aelys function values
//
// All Aelys-convention function values (closures, lambdas, named functions used as values) share a uniform fat pointer representation:
//
// { fn_ptr, env_ptr }.
//
// Non-capturing forms have env_ptr = null. C-convention FnPtrs stay as bare
// pointers; the C ABI has no notion of an env, and extern functions don't
// need one
//
// To make indirect calls uniform, every non-extern Aelys function receives an
// implicit `env: ptr` as its first LLVM parameter.
//
// Named functions ignore it (callers pass null); closure bodies use it to access captured values.
// The cost is zero under fastcc: it's just an unused register not a stack push
//
// Exception: closure functions already have `__env` as an explicit AIR-level parameter (added during lower_closure).
// These do not get the implicit env on top; function_has_implicit_env excludes them.
//
// At LLVM level, both forms end up with env at param 0, which is what indirect callers expect.
//
// The entry wrapper (__aelys_user_main) bridges from C convention to the Aelys main function by passing null as the env argument.

impl CodegenContext {
    pub(crate) fn declare_functions(&self, program: &AirProgram) -> Result<(), CodegenError> {
        self.ensure_no_reserved_bootstrap_builtins(program)?;

        for function in &program.functions {
            // Any C-convention entry point crosses the platform ABI boundary, even when
            // the function body lives in this module and is called via a fnptr later.
            if matches!(function.calling_conv, AirCallingConv::C) {
                reject_struct_abi_on_c_function(function, program)?;
            }

            let symbol_name = function_symbol_name(function);
            let fn_type = self.function_type(function, program)?;
            let fn_value = if let Some(existing) = self.module.get_function(&symbol_name) {
                existing
            } else {
                self.module.add_function(&symbol_name, fn_type, None)
            };

            fn_value.set_call_conventions(llvm_calling_convention(function.calling_conv));
            self.apply_function_attributes(fn_value, &function.attributes)?;

            if needs_sret(
                &function.ret_ty,
                function.calling_conv,
                self.target_is_windows(),
                program,
            ) {
                let ret_any_ty = air_type_to_llvm(&function.ret_ty, self.context)?;
                let sret_attr = self
                    .context
                    .create_type_attribute(Attribute::get_named_enum_kind_id("sret"), ret_any_ty);
                fn_value.add_attribute(AttributeLoc::Param(0), sret_attr);
            }
        }

        Ok(())
    }

    fn ensure_no_reserved_bootstrap_builtins(
        &self,
        program: &AirProgram,
    ) -> Result<(), CodegenError> {
        if let Some(function) = program
            .functions
            .iter()
            .find(|function| is_reserved_bootstrap_builtin(&function.name))
        {
            return Err(CodegenError::UnsupportedInstruction(
                reserved_bootstrap_builtin_message(&function.name),
            ));
        }
        Ok(())
    }

    pub(crate) fn define_function_bodies(&self, program: &AirProgram) -> Result<(), CodegenError> {
        let mut function_names = HashMap::with_capacity(program.functions.len());
        for function in &program.functions {
            function_names.insert(function.id, function_symbol_name(function));
        }

        for function in &program.functions {
            if function.is_extern {
                continue;
            }

            let symbol_name = function_symbol_name(function);
            let fn_value = self.module.get_function(&symbol_name).ok_or_else(|| {
                CodegenError::LlvmError(format!(
                    "missing declared LLVM function for {}",
                    function.name
                ))
            })?;

            let mut fcx = FunctionCodegen::new(
                self.context,
                &self.module,
                fn_value,
                function,
                program,
                &function_names,
            );
            fcx.generate()?;
        }

        Ok(())
    }

    pub(crate) fn emit_entry_wrapper(&self, program: &AirProgram) -> Result<(), CodegenError> {
        let Some(user_main) = program
            .functions
            .iter()
            .find(|function| !function.is_extern && function.name == "main")
        else {
            return Ok(());
        };

        if !user_main.params.is_empty() {
            return Err(CodegenError::InvalidNativeEntry(format!(
                "main must have no parameters (found {})",
                user_main.params.len()
            )));
        }

        if !matches!(user_main.ret_ty, AirType::Void | AirType::I64) {
            return Err(CodegenError::InvalidNativeEntry(format!(
                "main return type must be void or i64 (found {})",
                native_entry_type_name(&user_main.ret_ty)
            )));
        }

        if self.module.get_function(NATIVE_ENTRY_SYMBOL).is_some() {
            return Err(CodegenError::InvalidNativeEntry(format!(
                "symbol '{}' is reserved by the native runtime",
                NATIVE_ENTRY_SYMBOL
            )));
        }

        let user_symbol = function_symbol_name(user_main);
        let user_fn = self
            .module
            .get_function(&user_symbol)
            .ok_or_else(|| CodegenError::LlvmError(format!("missing function {}", user_symbol)))?;

        let wrapper_ty = self.context.i64_type().fn_type(&[], false);
        let wrapper = self
            .module
            .add_function(NATIVE_ENTRY_SYMBOL, wrapper_ty, None);
        wrapper.set_call_conventions(llvm_calling_convention(AirCallingConv::C));

        let builder = self.context.create_builder();
        let entry = self.context.append_basic_block(wrapper, "entry");
        builder.position_at_end(entry);
        // __aelys_main has an implicit env parameter; pass null.
        let null_env = self.context.ptr_type(AddressSpace::default()).const_null();
        let call = builder
            .build_call(user_fn, &[null_env.into()], "user_main")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        call.set_call_convention(user_fn.get_call_conventions());

        let return_value = if matches!(user_main.ret_ty, AirType::Void) {
            self.context.i64_type().const_zero()
        } else {
            call.try_as_basic_value()
                .basic()
                .ok_or_else(|| {
                    CodegenError::LlvmError(format!(
                        "function {} returned void for i64 native entry",
                        user_symbol
                    ))
                })?
                .into_int_value()
        };

        builder
            .build_return(Some(&return_value))
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        Ok(())
    }

    fn function_type(
        &self,
        function: &AirFunction,
        program: &AirProgram,
    ) -> Result<FunctionType<'static>, CodegenError> {
        let use_sret = needs_sret(
            &function.ret_ty,
            function.calling_conv,
            self.target_is_windows(),
            program,
        );
        let has_implicit_env = function_has_implicit_env(function);
        let extra = usize::from(use_sret) + usize::from(has_implicit_env);
        let mut params = Vec::with_capacity(function.params.len() + extra);
        if use_sret {
            params.push(self.context.ptr_type(AddressSpace::default()).into());
        }
        if has_implicit_env {
            // Implicit env pointer (ptr) as first user-visible param
            params.push(self.context.ptr_type(AddressSpace::default()).into());
        }
        for param in &function.params {
            let param_ty: BasicMetadataTypeEnum<'static> =
                air_basic_type_to_llvm(&param.ty, self.context)?.into();
            params.push(param_ty);
        }

        if matches!(function.ret_ty, AirType::Void) || use_sret {
            return Ok(self.context.void_type().fn_type(&params, false));
        }

        Ok(air_basic_type_to_llvm(&function.ret_ty, self.context)?.fn_type(&params, false))
    }

    fn apply_function_attributes(
        &self,
        function: FunctionValue<'static>,
        attrs: &FunctionAttribs,
    ) -> Result<(), CodegenError> {
        match attrs.inline {
            InlineHint::Default => {}
            InlineHint::Always => self.add_function_attribute(function, "alwaysinline")?,
            InlineHint::Never => self.add_function_attribute(function, "noinline")?,
        }

        if attrs.no_unwind {
            self.add_function_attribute(function, "nounwind")?;
        }

        if attrs.cold {
            self.add_function_attribute(function, "cold")?;
        }

        Ok(())
    }

    fn add_function_attribute(
        &self,
        function: FunctionValue<'static>,
        attribute_name: &str,
    ) -> Result<(), CodegenError> {
        let kind_id = Attribute::get_named_enum_kind_id(attribute_name);
        if kind_id == 0 {
            return Err(CodegenError::LlvmError(format!(
                "unknown LLVM function attribute: {}",
                attribute_name
            )));
        }

        let attr = self.context.create_enum_attribute(kind_id, 0);
        function.add_attribute(AttributeLoc::Function, attr);
        Ok(())
    }
}

/// Returns true if a function gets an implicit `env: ptr` prepended to its
/// LLVM parameter list.
///
/// This is every non-extern Aelys-convention function that doesn't already have `__env` as its first AIR parameter (closures)
///
/// The distinction matters ::
///
/// named functions get env added here (codegen-only, invisible in AIR), while closures already have it in their AIR param list
/// (added by lower_closure).
///
/// Both end up with env at LLVM param 0. Without this check, closures would get env twice and indirect calls would pass the wrong number of arguments.
pub(crate) fn function_has_implicit_env(function: &AirFunction) -> bool {
    if function.is_extern || !matches!(function.calling_conv, AirCallingConv::Aelys) {
        return false;
    }
    // Closures already declare __env as their first AIR param.
    !function.params.first().is_some_and(|p| p.name == "__env")
}

pub(crate) fn llvm_calling_convention(conv: AirCallingConv) -> u32 {
    match conv {
        AirCallingConv::Aelys => LLVMCallConv::LLVMFastCallConv as u32,
        AirCallingConv::C => LLVMCallConv::LLVMCCallConv as u32,
        AirCallingConv::Rust => LLVMCallConv::LLVMCCallConv as u32,
    }
}

pub(crate) fn function_symbol_name(function: &AirFunction) -> String {
    aelys_air::symbols::function_symbol_name(function)
}

/// Returns true if the type lowers to an aggregate that we must not pass
/// by value across the C ABI on Windows/MSVC.
/// Data enums count here because LLVM sees them as structs, not i32 tags.
pub(crate) fn is_abi_unsafe_type(ty: &AirType, program: &AirProgram) -> bool {
    matches!(
        ty,
        AirType::Str
            | AirType::Struct(_)
            | AirType::Slice(_)
            | AirType::Vec(_)
            | AirType::Array(_, _)
    ) || matches!(ty, AirType::Enum(name) if program
        .enums
        .iter()
        .find(|def| def.name == *name)
        .is_some_and(enum_has_data))
}

/// True when a function with this return type + calling convention needs sret
/// on the current target. Only C-convention functions need sret because
/// fastcc (Aelys-internal) is handled consistently by LLVM itself
pub(crate) fn needs_sret(
    ret_ty: &AirType,
    conv: AirCallingConv,
    is_windows: bool,
    program: &AirProgram,
) -> bool {
    is_windows && matches!(conv, AirCallingConv::C) && is_abi_unsafe_type(ret_ty, program)
}

fn reject_struct_abi_on_c_function(
    function: &AirFunction,
    program: &AirProgram,
) -> Result<(), CodegenError> {
    for param in &function.params {
        if matches!(&param.ty, AirType::Enum(name) if program
            .enums
            .iter()
            .find(|def| def.name == *name)
            .is_some_and(enum_has_data))
        {
            return Err(CodegenError::UnsupportedType(format!(
                "C-convention function '{}' has enum parameter '{}' (type {:?}), \
                 data enums must not cross the C ABI by value",
                function.name, param.name, param.ty
            )));
        }
        if is_abi_unsafe_type(&param.ty, program) {
            return Err(CodegenError::UnsupportedType(format!(
                "C-convention function '{}' has struct parameter '{}' (type {:?}), \
                 struct params must be flattened to scalars for C ABI compatibility",
                function.name, param.name, param.ty
            )));
        }
    }
    // struct returns are handled via sret in function_type() + declare_functions()
    Ok(())
}

fn native_entry_type_name(ty: &AirType) -> &'static str {
    match ty {
        AirType::I8 => "i8",
        AirType::I16 => "i16",
        AirType::I32 => "i32",
        AirType::I64 => "i64",
        AirType::U8 => "u8",
        AirType::U16 => "u16",
        AirType::U32 => "u32",
        AirType::U64 => "u64",
        AirType::F32 => "f32",
        AirType::F64 => "f64",
        AirType::Bool => "bool",
        AirType::Str => "string",
        AirType::Ptr(_) => "ptr",
        AirType::Struct(_) => "struct",
        AirType::Enum(_) => "enum",
        AirType::Array(_, _) => "array",
        AirType::Slice(_) => "slice",
        AirType::Vec(_) => "vec",
        AirType::FnPtr { .. } => "fn",
        AirType::Param(_) => "param",
        AirType::Opaque => "opaque",
        AirType::Void => "void",
    }
}

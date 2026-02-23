use crate::CodegenContext;
use crate::CodegenError;
use crate::body::FunctionCodegen;
use crate::{is_reserved_bootstrap_builtin, reserved_bootstrap_builtin_message};
use crate::types::air_basic_type_to_llvm;
use aelys_air::{
    AirFunction, AirProgram, AirType, CallingConv as AirCallingConv, FunctionAttribs, InlineHint,
};
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::llvm_sys::LLVMCallConv;
use inkwell::types::{BasicMetadataTypeEnum, BasicType, FunctionType};
use inkwell::values::FunctionValue;
use std::collections::HashMap;

impl CodegenContext {
    pub(crate) fn declare_functions(&self, program: &AirProgram) -> Result<(), CodegenError> {
        self.ensure_no_reserved_bootstrap_builtins(program)?;

        for function in &program.functions {
            let symbol_name = function_symbol_name(function);
            let fn_type = self.function_type(function)?;
            let fn_value = if let Some(existing) = self.module.get_function(&symbol_name) {
                existing
            } else {
                self.module.add_function(&symbol_name, fn_type, None)
            };

            fn_value.set_call_conventions(llvm_calling_convention(function.calling_conv));
            self.apply_function_attributes(fn_value, &function.attributes)?;
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
                CodegenError::LlvmError(format!("missing declared LLVM function for {}", function.name))
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
            return Err(CodegenError::UnsupportedInstruction(
                "main with parameters is not supported for LLVM native entry".to_string(),
            ));
        }

        if self.module.get_function("main").is_some() {
            return Ok(());
        }

        let user_symbol = function_symbol_name(user_main);
        let user_fn = self
            .module
            .get_function(&user_symbol)
            .ok_or_else(|| CodegenError::LlvmError(format!("missing function {}", user_symbol)))?;

        let wrapper_ty = self.context.i32_type().fn_type(&[], false);
        let wrapper = self.module.add_function("main", wrapper_ty, None);
        wrapper.set_call_conventions(llvm_calling_convention(AirCallingConv::C));

        let builder = self.context.create_builder();
        let entry = self.context.append_basic_block(wrapper, "entry");
        builder.position_at_end(entry);
        let call = builder
            .build_call(user_fn, &[], "user_main")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        call.set_call_convention(user_fn.get_call_conventions());
        builder
            .build_return(Some(&self.context.i32_type().const_zero()))
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        Ok(())
    }

    fn function_type(&self, function: &AirFunction) -> Result<FunctionType<'static>, CodegenError> {
        let mut params = Vec::with_capacity(function.params.len());
        for param in &function.params {
            let param_ty: BasicMetadataTypeEnum<'static> =
                air_basic_type_to_llvm(&param.ty, self.context)?.into();
            params.push(param_ty);
        }

        if matches!(function.ret_ty, AirType::Void) {
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

pub(crate) fn llvm_calling_convention(conv: AirCallingConv) -> u32 {
    match conv {
        AirCallingConv::Aelys => LLVMCallConv::LLVMFastCallConv as u32,
        AirCallingConv::C => LLVMCallConv::LLVMCCallConv as u32,
        AirCallingConv::Rust => LLVMCallConv::LLVMCCallConv as u32,
    }
}

pub(crate) fn function_symbol_name(function: &AirFunction) -> String {
    if !function.is_extern && function.name == "main" {
        "__aelys_main".to_string()
    } else {
        function.name.clone()
    }
}

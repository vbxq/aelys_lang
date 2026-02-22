pub mod types;

use crate::types::air_basic_type_to_llvm;
use aelys_air::{
    AirFunction, AirProgram, CallingConv as AirCallingConv, FunctionAttribs, InlineHint,
};
use inkwell::attributes::{Attribute, AttributeLoc};
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::llvm_sys::LLVMCallConv;
use inkwell::module::Module;
use inkwell::types::{BasicMetadataTypeEnum, BasicType};
use inkwell::values::FunctionValue;

pub struct CodegenContext {
    context: &'static Context,
    module: Module<'static>,
    builder: Builder<'static>,
}

impl CodegenContext {
    pub fn new(module_name: &str) -> Self {
        let context = Box::leak(Box::new(Context::create()));
        let module = context.create_module(module_name);
        let builder = context.create_builder();
        Self {
            context,
            module,
            builder,
        }
    }

    pub fn compile(&mut self, program: &AirProgram) -> Result<(), CodegenError> {
        let _ = self.builder.get_insert_block();
        self.declare_struct_types(program)?;
        self.declare_functions(program)?;
        Ok(())
    }

    pub fn emit_object(&self, path: &str) -> Result<(), CodegenError> {
        let _ = path;
        todo!()
    }

    pub fn emit_ir(&self, path: &str) -> Result<(), CodegenError> {
        self.module
            .print_to_file(path)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))
    }

    fn declare_struct_types(&self, program: &AirProgram) -> Result<(), CodegenError> {
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

        Ok(())
    }

    fn declare_functions(&self, program: &AirProgram) -> Result<(), CodegenError> {
        for function in &program.functions {
            let fn_type = self.function_type(function)?;
            let fn_value = if let Some(existing) = self.module.get_function(&function.name) {
                existing
            } else {
                self.module.add_function(&function.name, fn_type, None)
            };

            fn_value.set_call_conventions(llvm_calling_convention(function.calling_conv));
            self.apply_function_attributes(fn_value, &function.attributes)?;

            if function.is_extern {
                continue;
            }
        }

        Ok(())
    }

    fn function_type(
        &self,
        function: &AirFunction,
    ) -> Result<inkwell::types::FunctionType<'static>, CodegenError> {
        let mut params: Vec<BasicMetadataTypeEnum<'static>> =
            Vec::with_capacity(function.params.len());
        for param in &function.params {
            params.push(air_basic_type_to_llvm(&param.ty, self.context)?.into());
        }

        if matches!(function.ret_ty, aelys_air::AirType::Void) {
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

fn llvm_calling_convention(conv: AirCallingConv) -> u32 {
    match conv {
        AirCallingConv::Aelys => LLVMCallConv::LLVMFastCallConv as u32,
        AirCallingConv::C => LLVMCallConv::LLVMCCallConv as u32,
        AirCallingConv::Rust => LLVMCallConv::LLVMCCallConv as u32,
    }
}

#[derive(Debug)]
pub enum CodegenError {
    LlvmError(String),
    UnsupportedType(String),
    UnsupportedInstruction(String),
}

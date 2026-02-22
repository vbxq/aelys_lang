use crate::CodegenError;
use crate::body::FunctionCodegen;
use crate::functions::llvm_calling_convention;
use crate::types::air_basic_type_to_llvm;
use aelys_air::{AirType, Callee, LocalId, Operand};
use inkwell::types::{BasicMetadataTypeEnum, BasicType, FunctionType};
use inkwell::values::{BasicMetadataValueEnum, BasicValueEnum, FunctionValue};

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn generate_call(
        &mut self,
        callee: &Callee,
        args: &[Operand],
        expected_ret: Option<&AirType>,
    ) -> Result<Option<BasicValueEnum<'static>>, CodegenError> {
        let mut arg_values = Vec::with_capacity(args.len());
        for arg in args {
            arg_values.push(self.generate_operand(arg)?);
        }

        let metadata_args: Vec<BasicMetadataValueEnum<'static>> =
            arg_values.iter().copied().map(Into::into).collect();
        let arg_types: Vec<BasicMetadataTypeEnum<'static>> =
            arg_values.iter().map(|v| v.get_type().into()).collect();

        match callee {
            Callee::FnPtr(local) => {
                let fn_ptr = self.load_local(*local)?.into_pointer_value();
                let fn_ty = self.fn_ptr_type_for_local(*local)?;
                let call = self
                    .builder
                    .build_indirect_call(fn_ty, fn_ptr, &metadata_args, "call_indirect")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                Ok(call.try_as_basic_value().basic())
            }
            _ => {
                let fn_value = self.resolve_callee(callee, &arg_types, expected_ret)?;
                let call = self
                    .builder
                    .build_call(fn_value, &metadata_args, "call_direct")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                Ok(call.try_as_basic_value().basic())
            }
        }
    }

    fn resolve_callee(
        &mut self,
        callee: &Callee,
        arg_types: &[BasicMetadataTypeEnum<'static>],
        expected_ret: Option<&AirType>,
    ) -> Result<FunctionValue<'static>, CodegenError> {
        match callee {
            Callee::Direct(id) => {
                let name = self.function_names.get(id).ok_or_else(|| {
                    CodegenError::UnsupportedInstruction(format!("unknown function id {}", id.0))
                })?;
                self.module
                    .get_function(name)
                    .ok_or_else(|| CodegenError::LlvmError(format!("missing function {}", name)))
            }
            Callee::Named(name) => {
                if let Some(function) = self.module.get_function(name) {
                    return Ok(function);
                }
                let fn_ty = self.ad_hoc_function_type(arg_types, expected_ret)?;
                Ok(self.module.add_function(name, fn_ty, None))
            }
            Callee::Extern(name, conv) => {
                let function = if let Some(function) = self.module.get_function(name) {
                    function
                } else {
                    let fn_ty = self.ad_hoc_function_type(arg_types, expected_ret)?;
                    self.module.add_function(name, fn_ty, None)
                };
                function.set_call_conventions(llvm_calling_convention(*conv));
                Ok(function)
            }
            Callee::FnPtr(_) => Err(CodegenError::UnsupportedInstruction(
                "callee::FnPtr should be handled by generate_call".to_string(),
            )),
        }
    }

    fn ad_hoc_function_type(
        &self,
        arg_types: &[BasicMetadataTypeEnum<'static>],
        expected_ret: Option<&AirType>,
    ) -> Result<FunctionType<'static>, CodegenError> {
        match expected_ret {
            None | Some(AirType::Void) => Ok(self.context.void_type().fn_type(arg_types, false)),
            Some(ret) => Ok(air_basic_type_to_llvm(ret, self.context)?.fn_type(arg_types, false)),
        }
    }

    fn fn_ptr_type_for_local(&self, local: LocalId) -> Result<FunctionType<'static>, CodegenError> {
        match self.local_air_type(local)? {
            AirType::FnPtr { params, ret, .. } => {
                let mut param_types = Vec::with_capacity(params.len());
                for param in params {
                    param_types.push(air_basic_type_to_llvm(param, self.context)?.into());
                }

                match ret.as_ref() {
                    AirType::Void => Ok(self.context.void_type().fn_type(&param_types, false)),
                    other => Ok(air_basic_type_to_llvm(other, self.context)?.fn_type(&param_types, false)),
                }
            }
            other => Err(CodegenError::UnsupportedType(format!(
                "local {} is not fn ptr: {:?}",
                local.0, other
            ))),
        }
    }
}

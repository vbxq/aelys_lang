use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::lowering::functions::llvm_calling_convention;
use crate::types::{aelys_string_type, air_basic_type_to_llvm};
use crate::{is_reserved_bootstrap_builtin, reserved_bootstrap_builtin_message};
use aelys_air::{AirConst, AirType, Callee, LocalId, Operand};
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

        // During bootstrap, print/println are reserved names lowered to __aelys_write.
        // Declaration phase rejects user definitions with these names.
        if let Callee::Named(name) = callee
            && is_reserved_bootstrap_builtin(name)
        {
            debug_assert!(
                self.module.get_function(name).is_none(),
                "reserved builtin must be rejected during declaration phase"
            );
            if self.module.get_function(name).is_some() {
                return Err(CodegenError::UnsupportedInstruction(
                    reserved_bootstrap_builtin_message(name),
                ));
            }
            return self.generate_bootstrap_print_call(
                name == "println",
                args,
                &arg_values,
                expected_ret,
            );
        }

        let metadata_args: Vec<BasicMetadataValueEnum<'static>> =
            arg_values.iter().copied().map(Into::into).collect();
        let arg_types: Vec<BasicMetadataTypeEnum<'static>> =
            arg_values.iter().map(|v| v.get_type().into()).collect();

        match callee {
            Callee::FnPtr(local) => {
                let fn_ptr = self.load_local(*local)?.into_pointer_value();
                let (fn_ty, call_conv) = self.fn_ptr_signature_for_local(*local)?;
                let call = self
                    .builder
                    .build_indirect_call(fn_ty, fn_ptr, &metadata_args, "call_indirect")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                call.set_call_convention(call_conv);
                Ok(call.try_as_basic_value().basic())
            }
            _ => {
                let fn_value = self.resolve_callee(callee, &arg_types, expected_ret)?;
                let call = self
                    .builder
                    .build_call(fn_value, &metadata_args, "call_direct")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                call.set_call_convention(fn_value.get_call_conventions());
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

    fn generate_bootstrap_print_call(
        &mut self,
        newline: bool,
        args: &[Operand],
        arg_values: &[BasicValueEnum<'static>],
        expected_ret: Option<&AirType>,
    ) -> Result<Option<BasicValueEnum<'static>>, CodegenError> {
        if args.len() != 1 || arg_values.len() != 1 {
            return Err(CodegenError::UnsupportedInstruction(
                "print/println expects exactly one argument".to_string(),
            ));
        }

        // bootstrap: move to stdlib when ready
        let (ptr, len) = match (&args[0], arg_values[0]) {
            (Operand::Const(AirConst::Str(text)), _) => {
                let (ptr, len) = self.global_string_ptr_len(text)?;
                (ptr, self.context.i64_type().const_int(len, false))
            }
            (_, value) if value.is_struct_value() => {
                let struct_value = value.into_struct_value();
                if struct_value.get_type() != aelys_string_type(self.context) {
                    return Err(CodegenError::UnsupportedType(
                        "print/println currently expects a string argument".to_string(),
                    ));
                }
                let (ptr, len) = self.string_parts_from_value(struct_value)?;
                (ptr, len)
            }
            _ => {
                return Err(CodegenError::UnsupportedType(
                    "print/println currently expects a string argument".to_string(),
                ));
            }
        };

        let write_fn = self.ensure_write_function();
        self.builder
            .build_call(write_fn, &[ptr.into(), len.into()], "")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        if newline {
            let (nl_ptr, nl_len) = self.global_string_ptr_len("\n")?;
            let nl_len = self.context.i64_type().const_int(nl_len, false);
            self.builder
                .build_call(write_fn, &[nl_ptr.into(), nl_len.into()], "")
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        }

        match expected_ret {
            None | Some(AirType::Void) => Ok(None),
            Some(ret) => Ok(Some(
                air_basic_type_to_llvm(ret, self.context)?.const_zero(),
            )),
        }
    }

    fn fn_ptr_signature_for_local(
        &self,
        local: LocalId,
    ) -> Result<(FunctionType<'static>, u32), CodegenError> {
        match self.local_air_type(local)? {
            AirType::FnPtr { params, ret, conv } => {
                let mut param_types = Vec::with_capacity(params.len());
                for param in params {
                    param_types.push(air_basic_type_to_llvm(param, self.context)?.into());
                }

                let fn_ty = match ret.as_ref() {
                    AirType::Void => self.context.void_type().fn_type(&param_types, false),
                    other => {
                        air_basic_type_to_llvm(other, self.context)?.fn_type(&param_types, false)
                    }
                };
                Ok((fn_ty, llvm_calling_convention(*conv)))
            }
            other => Err(CodegenError::UnsupportedType(format!(
                "local {} is not fn ptr: {:?}",
                local.0, other
            ))),
        }
    }
}

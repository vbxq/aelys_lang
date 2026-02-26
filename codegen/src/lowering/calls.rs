use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::lowering::functions::{llvm_calling_convention, needs_sret};
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

        // print/println are reserved bootstrap names lowered to __aelys_write
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
                if let Some(ret_air_ty) = expected_ret
                    && self.callee_needs_sret(callee)
                {
                    let ret_ty = air_basic_type_to_llvm(ret_air_ty, self.context)?;
                    Ok(Some(self.call_with_sret(
                        fn_value,
                        &metadata_args,
                        ret_ty,
                        "call_sret",
                    )?))
                } else {
                    let call = self
                        .builder
                        .build_call(fn_value, &metadata_args, "call_direct")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                    call.set_call_convention(fn_value.get_call_conventions());
                    Ok(call.try_as_basic_value().basic())
                }
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

        let arg_type = self.operand_type(&args[0])?;
        let value = arg_values[0];

        let string_value = match arg_type {
            AirType::I64 | AirType::I32 | AirType::I16 | AirType::I8 => {
                let int_val = value.into_int_value();
                let i64_val = if int_val.get_type() == self.context.i64_type() {
                    int_val
                } else {
                    self.builder
                        .build_int_s_extend(int_val, self.context.i64_type(), "ext_i64")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))?
                };
                let fn_val = self.ensure_to_string_i64_function();
                self.call_sret_returning_fn(fn_val, &[i64_val.into()], "to_str")?
            }
            AirType::U8 | AirType::U16 | AirType::U32 | AirType::U64 => {
                // unsigned: zero-extend to i64 before calling to_string_i64
                let int_val = value.into_int_value();
                let i64_val = if int_val.get_type() == self.context.i64_type() {
                    int_val
                } else {
                    self.builder
                        .build_int_z_extend(int_val, self.context.i64_type(), "zext_i64")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))?
                };
                let fn_val = self.ensure_to_string_i64_function();
                self.call_sret_returning_fn(fn_val, &[i64_val.into()], "to_str")?
            }
            AirType::F64 | AirType::F32 => {
                let float_val = value.into_float_value();
                let f64_val = if float_val.get_type() == self.context.f64_type() {
                    float_val
                } else {
                    self.builder
                        .build_float_ext(float_val, self.context.f64_type(), "ext_f64")
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))?
                };
                let fn_val = self.ensure_to_string_f64_function();
                self.call_sret_returning_fn(fn_val, &[f64_val.into()], "to_str")?
            }
            AirType::Bool => {
                let bool_val = value.into_int_value();
                let i64_val = self
                    .builder
                    .build_int_z_extend(bool_val, self.context.i64_type(), "bool_to_i64")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                let fn_val = self.ensure_to_string_bool_function();
                self.call_sret_returning_fn(fn_val, &[i64_val.into()], "to_str")?
            }
            AirType::Str => match &args[0] {
                Operand::Const(AirConst::Str(text)) => self.global_string_value(text)?,
                _ if value.is_struct_value() => {
                    let struct_val = value.into_struct_value();
                    if struct_val.get_type() != aelys_string_type(self.context) {
                        return Err(CodegenError::UnsupportedType(
                            "expected string struct value".to_string(),
                        ));
                    }
                    value
                }
                _ => {
                    return Err(CodegenError::UnsupportedType(
                        "expected string value".to_string(),
                    ));
                }
            },
            _ => {
                return Err(CodegenError::UnsupportedType(format!(
                    "print/println does not support type {:?}",
                    arg_type
                )));
            }
        };

        let (ptr, len) = if string_value.is_struct_value() {
            self.string_parts_from_value(string_value.into_struct_value())?
        } else {
            return Err(CodegenError::LlvmError(
                "to_string did not return struct".to_string(),
            ));
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

        // print/println is semantically void but sema infers Dynamic → I64 for its return,
        // so the AIR may emit Rvalue::Call (not CallVoid). We return const_zero() here because
        // erroring would break normal println("hi") calls. The real fix is in sema: type
        // bootstrap builtins as void so the AIR always emits CallVoid
        match expected_ret {
            None | Some(AirType::Void) => Ok(None),
            Some(ret) => Ok(Some(
                air_basic_type_to_llvm(ret, self.context)?.const_zero(),
            )),
        }
    }

    /// Check if a callee uses sret convention on the current target.
    /// Only C-convention functions with struct-like returns need this.
    fn callee_needs_sret(&self, callee: &Callee) -> bool {
        let is_windows = self.target_is_windows();
        match callee {
            Callee::Direct(id) => self
                .program
                .functions
                .iter()
                .find(|f| f.id == *id)
                .map_or(false, |f| {
                    needs_sret(&f.ret_ty, f.calling_conv, is_windows)
                }),
            Callee::Extern(name, _) => self
                .program
                .functions
                .iter()
                .find(|f| f.name == *name && f.is_extern)
                .map_or(false, |f| {
                    needs_sret(&f.ret_ty, f.calling_conv, is_windows)
                }),
            _ => false,
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

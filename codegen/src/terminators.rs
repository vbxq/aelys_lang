use crate::CodegenError;
use crate::body::FunctionCodegen;
use crate::operands::{constant_kind_name, is_signed_int_size};
use crate::types::air_basic_type_to_llvm;
use aelys_air::{AirConst, AirTerminator, AirType};
use inkwell::values::IntValue;

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn generate_terminator(&mut self, term: &AirTerminator) -> Result<(), CodegenError> {
        match term {
            AirTerminator::Return(Some(operand)) => {
                let value = self.generate_operand(operand)?;
                self.builder
                    .build_return(Some(&value))
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                Ok(())
            }
            AirTerminator::Return(None) => {
                if matches!(self.air_function.ret_ty, AirType::Void) {
                    self.builder
                        .build_return(None)
                        .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                    return Ok(());
                }

                let ret_ty = air_basic_type_to_llvm(&self.air_function.ret_ty, self.context)?;
                let zero = ret_ty.const_zero();
                self.builder
                    .build_return(Some(&zero))
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                Ok(())
            }
            AirTerminator::Goto(block) => {
                self.builder
                    .build_unconditional_branch(self.lookup_block(*block)?)
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                Ok(())
            }
            AirTerminator::Branch {
                cond,
                then_block,
                else_block,
            } => {
                let cond_value = self.generate_operand(cond)?.into_int_value();
                self.builder
                    .build_conditional_branch(
                        cond_value,
                        self.lookup_block(*then_block)?,
                        self.lookup_block(*else_block)?,
                    )
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                Ok(())
            }
            AirTerminator::Unreachable => {
                self.builder
                    .build_unreachable()
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                Ok(())
            }
            AirTerminator::Panic { message, .. } => {
                let panic_fn = self.ensure_panic_function();
                let msg_ptr = self.global_string_ptr(message)?;
                self.builder
                    .build_call(panic_fn, &[msg_ptr.into()], "")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                self.builder
                    .build_unreachable()
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                Ok(())
            }
            AirTerminator::Switch {
                discr,
                targets,
                default,
            } => {
                let discr_value = self.generate_operand(discr)?.into_int_value();
                let mut cases = Vec::with_capacity(targets.len());
                for (constant, block) in targets {
                    cases.push((
                        self.switch_case_value(discr_value, constant)?,
                        self.lookup_block(*block)?,
                    ));
                }

                self.builder
                    .build_switch(discr_value, self.lookup_block(*default)?, &cases)
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
                Ok(())
            }
            AirTerminator::Invoke { .. } | AirTerminator::Unwind => todo!(),
        }
    }

    fn switch_case_value(
        &self,
        discr: IntValue<'static>,
        constant: &AirConst,
    ) -> Result<IntValue<'static>, CodegenError> {
        let ty = discr.get_type();
        match constant {
            AirConst::IntLiteral(v) => Ok(ty.const_int(*v as u64, true)),
            AirConst::Int(v, size) => Ok(ty.const_int(*v as u64, is_signed_int_size(*size))),
            AirConst::Bool(v) => Ok(ty.const_int(u64::from(*v), false)),
            other => Err(CodegenError::UnsupportedInstruction(format!(
                "unsupported switch constant kind: {}",
                constant_kind_name(other)
            ))),
        }
    }
}

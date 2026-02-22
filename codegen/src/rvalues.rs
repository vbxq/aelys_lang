use crate::CodegenError;
use crate::body::FunctionCodegen;
use crate::types::air_basic_type_to_llvm;
use aelys_air::{AirType, Rvalue};
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
                self.builder
                    .build_load(inner_ty, ptr, "deref")
                    .map_err(|e| CodegenError::LlvmError(e.to_string()))
            }
            Rvalue::Cast { operand, from, to } => self.generate_cast(operand, from, to),
            Rvalue::Discriminant(_) => todo!(),
        }
    }
}

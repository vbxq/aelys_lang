use crate::CodegenError;
use crate::body::FunctionCodegen;
use aelys_air::AirType;
use inkwell::values::{BasicValueEnum, PointerValue};

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn store_value(
        &self,
        ptr: PointerValue<'static>,
        value: BasicValueEnum<'static>,
        ty: &AirType,
    ) -> Result<(), CodegenError> {
        let store = self
            .builder
            .build_store(ptr, value)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        let align = alignment_for_type(self, ty)?;
        store
            .set_alignment(align)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        Ok(())
    }

    pub(crate) fn air_type_size(&self, ty: &AirType) -> Result<u32, CodegenError> {
        self.type_size_align(ty).map(|(size, _)| size)
    }

    fn type_size_align(&self, ty: &AirType) -> Result<(u32, u32), CodegenError> {
        match ty {
            AirType::I8 | AirType::U8 | AirType::Bool => Ok((1, 1)),
            AirType::I16 | AirType::U16 => Ok((2, 2)),
            AirType::I32 | AirType::U32 | AirType::F32 => Ok((4, 4)),
            AirType::I64 | AirType::U64 | AirType::F64 => Ok((8, 8)),
            AirType::Ptr(_) | AirType::Str | AirType::FnPtr { .. } => Ok((8, 8)),
            AirType::Void => Ok((0, 1)),
            AirType::Slice(_) => Ok((16, 8)),
            AirType::Array(inner, n) => {
                let (size, align) = self.type_size_align(inner)?;
                Ok((size.saturating_mul(*n as u32), align))
            }
            AirType::Struct(name) => {
                let def = self
                    .program
                    .structs
                    .iter()
                    .find(|s| s.name == *name)
                    .ok_or_else(|| CodegenError::UnsupportedType(format!("unknown struct {}", name)))?;
                if def.fields.is_empty() {
                    return Ok((0, 1));
                }
                if def.fields.iter().all(|f| f.offset.is_some()) {
                    let mut max_align = 1u32;
                    let mut end = 0u32;
                    for field in &def.fields {
                        let (fs, fa) = self.type_size_align(&field.ty)?;
                        max_align = max_align.max(fa);
                        end = end.max(field.offset.unwrap_or(0).saturating_add(fs));
                    }
                    return Ok((align_to(end, max_align), max_align));
                }

                let mut offset = 0u32;
                let mut max_align = 1u32;
                for field in &def.fields {
                    let (fs, fa) = self.type_size_align(&field.ty)?;
                    offset = align_to(offset, fa);
                    offset = offset.saturating_add(fs);
                    max_align = max_align.max(fa);
                }
                Ok((align_to(offset, max_align), max_align))
            }
            AirType::Param(id) => Err(CodegenError::UnsupportedType(format!(
                "unexpected unresolved type parameter {:?}",
                id
            ))),
        }
    }
}

pub(crate) fn alignment_for_type(
    fcx: &FunctionCodegen<'_>,
    ty: &AirType,
) -> Result<u32, CodegenError> {
    let (_, align) = fcx.type_size_align(ty)?;
    Ok(align.max(1))
}

fn align_to(offset: u32, align: u32) -> u32 {
    if align == 0 {
        return offset;
    }
    (offset + align - 1) & !(align - 1)
}

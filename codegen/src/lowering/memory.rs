use crate::CodegenError;
use crate::lowering::body::FunctionCodegen;
use crate::types::alignment_of;
use aelys_air::AirType;
use inkwell::types::{BasicType, BasicTypeEnum};
use inkwell::values::{
    BasicValue, BasicValueEnum, FunctionValue, InstructionOpcode, InstructionValue, MetadataValue,
    PointerValue,
};

const AGGREGATE_COPY_LEAVES: u64 = 32;

impl<'a> FunctionCodegen<'a> {
    fn vec_header_scope(&self) -> MetadataValue<'static> {
        let ctx = self.context;
        let domain = ctx.metadata_node(&[ctx.metadata_string("aelys").into()]);
        let scope = ctx.metadata_node(&[
            ctx.metadata_string("aelys.vec.header").into(),
            domain.into(),
        ]);
        ctx.metadata_node(&[scope.into()])
    }

    pub(crate) fn mark_vec_header_load(
        &self,
        load: BasicValueEnum<'static>,
    ) -> Result<(), CodegenError> {
        if let Some(inst) = load.as_instruction_value() {
            inst.set_metadata(
                self.vec_header_scope(),
                self.context.get_kind_id("alias.scope"),
            )
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        }
        Ok(())
    }

    // a scalar slot is never a vec header, so llvm may keep headers in registers across this store
    pub(crate) fn mark_scalar_elem_store(
        &self,
        store: InstructionValue<'static>,
    ) -> Result<(), CodegenError> {
        store
            .set_metadata(self.vec_header_scope(), self.context.get_kind_id("noalias"))
            .map_err(|e| CodegenError::LlvmError(e.to_string()))
    }

    pub(crate) fn store_value(
        &self,
        ptr: PointerValue<'static>,
        value: BasicValueEnum<'static>,
    ) -> Result<Option<InstructionValue<'static>>, CodegenError> {
        if let Some((src, align)) = self.forwardable_aggregate_load(value) {
            let size = value
                .get_type()
                .size_of()
                .ok_or_else(|| CodegenError::LlvmError("unsized aggregate copy".to_string()))?;
            self.builder
                .build_memmove(ptr, align, src, align, size)
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
            return Ok(None);
        }
        let store = self
            .builder
            .build_store(ptr, value)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        store
            .set_alignment(alignment_of(value.get_type()))
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        Ok(Some(store))
    }

    // llvm splits an aggregate load and store into one node per scalar, which takes minutes on a large array
    fn forwardable_aggregate_load(
        &self,
        value: BasicValueEnum<'static>,
    ) -> Option<(PointerValue<'static>, u32)> {
        if leaves(value.get_type()) < AGGREGATE_COPY_LEAVES {
            return None;
        }
        let load = value.as_instruction_value()?;
        if load.get_opcode() != InstructionOpcode::Load
            || load.get_parent()? != self.builder.get_insert_block()?
        {
            return None;
        }
        let mut next = load.get_next_instruction();
        while let Some(inst) = next {
            if matches!(
                inst.get_opcode(),
                InstructionOpcode::Store
                    | InstructionOpcode::Call
                    | InstructionOpcode::Invoke
                    | InstructionOpcode::AtomicRMW
                    | InstructionOpcode::AtomicCmpXchg
                    | InstructionOpcode::Fence
            ) {
                return None;
            }
            next = inst.get_next_instruction();
        }
        let src = load.get_operand(0)?.value()?.into_pointer_value();
        Some((src, load.get_alignment().ok()?))
    }

    pub(crate) fn load_value(
        &self,
        ty: BasicTypeEnum<'static>,
        ptr: PointerValue<'static>,
        name: &str,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let value = self
            .builder
            .build_load(ty, ptr, name)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        if let Some(instruction) = value.as_instruction_value() {
            instruction
                .set_alignment(alignment_of(ty))
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        }
        Ok(value)
    }

    pub(crate) fn entry_alloca(
        &mut self,
        ty: BasicTypeEnum<'static>,
        name: &str,
    ) -> Result<PointerValue<'static>, CodegenError> {
        let entry = self.entry_block()?;
        let resume = self.builder.get_insert_block();
        match entry.get_first_instruction() {
            Some(first) => self.builder.position_before(&first),
            None => self.builder.position_at_end(entry),
        }
        let ptr = self
            .builder
            .build_alloca(ty, name)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        self.align_alloca(ptr, ty)?;
        if let Some(block) = resume {
            self.builder.position_at_end(block);
        }
        Ok(ptr)
    }

    pub(crate) fn align_alloca(
        &self,
        ptr: PointerValue<'static>,
        ty: BasicTypeEnum<'static>,
    ) -> Result<(), CodegenError> {
        if let Some(inst) = ptr.as_instruction() {
            inst.set_alignment(alignment_of(ty))
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        }
        Ok(())
    }

    pub(crate) fn air_type_size(&self, ty: &AirType) -> Result<u32, CodegenError> {
        self.type_size_align(ty).map(|(size, _)| size)
    }

    fn type_size_align(&self, ty: &AirType) -> Result<(u32, u32), CodegenError> {
        match ty {
            AirType::I8 | AirType::U8 | AirType::Bool => Ok((1, 1)),
            AirType::I16 | AirType::U16 => Ok((2, 2)),
            AirType::I32 | AirType::U32 | AirType::F32 | AirType::Char => Ok((4, 4)),
            AirType::I64 | AirType::U64 | AirType::F64 => Ok((8, 8)),
            AirType::Ptr(inner) if aelys_air::objects::dyn_trait(inner).is_some() => Ok((16, 8)),
            AirType::Ptr(_) | AirType::Rc(_) => Ok((8, 8)),
            AirType::FnPtr { conv, .. } => {
                if matches!(conv, aelys_air::CallingConv::Aelys) {
                    Ok((16, 8))
                } else {
                    Ok((8, 8))
                }
            }
            AirType::Str => Ok((16, 8)),
            AirType::Void => Ok((0, 1)),
            AirType::Slice(_) => Ok((16, 8)),
            AirType::Vec(_) => Ok((24, 8)),
            AirType::Array(inner, n) => {
                let (size, align) = self.type_size_align(inner)?;
                Ok((size.saturating_mul(*n as u32), align))
            }
            AirType::Enum(r) => {
                if let Some(layout) = self.program.struct_sizes.get(r.symbol().as_str()) {
                    Ok((layout.size, layout.align))
                } else {
                    Ok((4, 4))
                }
            }
            AirType::Struct(r) => {
                let name = r.symbol();
                let def = self
                    .program
                    .structs
                    .iter()
                    .find(|s| s.name == name)
                    .ok_or_else(|| {
                        CodegenError::UnsupportedType(format!("unknown struct {}", name))
                    })?;
                if def.fields.is_empty() {
                    return Ok((0, 1));
                }
                // codegen must never compute layout, that's air's job.
                if !def.fields.iter().all(|f| f.offset.is_some()) {
                    return Err(CodegenError::UnsupportedType(format!(
                        "struct `{}` has uncomputed field offsets; AIR layout pass was not run",
                        name
                    )));
                }
                let mut max_align = 1u32;
                let mut end = 0u32;
                for field in &def.fields {
                    let (fs, fa) = self.type_size_align(&field.ty)?;
                    max_align = max_align.max(fa);
                    end = end.max(
                        field
                            .offset
                            .expect("offset checked above")
                            .saturating_add(fs),
                    );
                }
                Ok((align_to(end, max_align), max_align))
            }
            AirType::Param(id) => Err(CodegenError::UnsupportedType(format!(
                "unexpected unresolved type parameter {:?}",
                id
            ))),
            AirType::Opaque => Err(CodegenError::UnsupportedType(
                "unresolved Dynamic type reached codegen".to_string(),
            )),
        }
    }
}

fn align_to(offset: u32, align: u32) -> u32 {
    if align == 0 {
        return offset;
    }
    (offset + align - 1) & !(align - 1)
}

fn leaves(ty: BasicTypeEnum<'static>) -> u64 {
    match ty {
        BasicTypeEnum::ArrayType(array) => {
            u64::from(array.len()).saturating_mul(leaves(array.get_element_type()))
        }
        BasicTypeEnum::StructType(fields) => fields.get_field_types().into_iter().map(leaves).sum(),
        _ => 1,
    }
}

// the load a forwarded aggregate copy leaves behind would still cost llvm its per-scalar split
pub(crate) fn drop_dead_aggregate_loads(function: FunctionValue<'static>) {
    for block in function.get_basic_block_iter() {
        let mut next = block.get_first_instruction();
        while let Some(inst) = next {
            next = inst.get_next_instruction();
            if inst.get_opcode() == InstructionOpcode::Load
                && inst.get_first_use().is_none()
                && matches!(
                    BasicTypeEnum::try_from(inst.get_type()),
                    Ok(BasicTypeEnum::ArrayType(_) | BasicTypeEnum::StructType(_))
                )
            {
                inst.erase_from_basic_block();
            }
        }
    }
}

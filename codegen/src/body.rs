use crate::CodegenError;
use crate::layout::alignment_for_type;
use crate::types::air_basic_type_to_llvm;
use aelys_air::{AirFunction, AirProgram, AirType, BlockId, FunctionId, LocalId};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use std::collections::HashMap;

pub(crate) struct FunctionCodegen<'a> {
    pub(crate) context: &'static Context,
    pub(crate) module: &'a Module<'static>,
    pub(crate) builder: Builder<'static>,
    pub(crate) function: FunctionValue<'static>,
    pub(crate) air_function: &'a AirFunction,
    pub(crate) program: &'a AirProgram,
    pub(crate) function_names: &'a HashMap<FunctionId, String>,
    pub(crate) block_map: HashMap<BlockId, BasicBlock<'static>>,
    pub(crate) local_allocas: HashMap<LocalId, PointerValue<'static>>,
    pub(crate) local_types: HashMap<LocalId, AirType>,
    pub(crate) string_id: u64,
}

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn new(
        context: &'static Context,
        module: &'a Module<'static>,
        function: FunctionValue<'static>,
        air_function: &'a AirFunction,
        program: &'a AirProgram,
        function_names: &'a HashMap<FunctionId, String>,
    ) -> Self {
        let mut local_types = HashMap::new();
        for param in &air_function.params {
            local_types.insert(param.id, param.ty.clone());
        }
        for local in &air_function.locals {
            local_types.insert(local.id, local.ty.clone());
        }

        Self {
            context,
            module,
            builder: context.create_builder(),
            function,
            air_function,
            program,
            function_names,
            block_map: HashMap::new(),
            local_allocas: HashMap::new(),
            local_types,
            string_id: 0,
        }
    }

    pub(crate) fn generate(&mut self) -> Result<(), CodegenError> {
        self.create_blocks();
        self.create_allocas()?;
        self.copy_params()?;
        self.generate_blocks()
    }

    fn create_blocks(&mut self) {
        for block in &self.air_function.blocks {
            let name = format!("bb{}", block.id.0);
            let llvm_block = self.context.append_basic_block(self.function, &name);
            self.block_map.insert(block.id, llvm_block);
        }
    }

    fn create_allocas(&mut self) -> Result<(), CodegenError> {
        let entry = self.entry_block()?;
        self.builder.position_at_end(entry);

        let locals: Vec<_> = self
            .air_function
            .locals
            .iter()
            .map(|local| (local.id, local.ty.clone()))
            .collect();
        for (id, ty) in locals {
            self.ensure_local_alloca(id, &ty)?;
        }

        Ok(())
    }

    fn copy_params(&mut self) -> Result<(), CodegenError> {
        let params = self.air_function.params.clone();
        for (index, param) in params.iter().enumerate() {
            let value = self.function.get_nth_param(index as u32).ok_or_else(|| {
                CodegenError::LlvmError(format!(
                    "missing LLVM param {} in `{}`",
                    index, self.air_function.name
                ))
            })?;
            let ptr = self.lookup_local_ptr(param.id)?;
            self.store_value(ptr, value.into(), &param.ty)?;
        }

        Ok(())
    }

    fn generate_blocks(&mut self) -> Result<(), CodegenError> {
        let blocks = self.air_function.blocks.clone();
        for block in &blocks {
            let llvm_block = self.lookup_block(block.id)?;
            self.builder.position_at_end(llvm_block);
            for stmt in &block.stmts {
                self.generate_stmt(&stmt.kind)?;
            }
            self.generate_terminator(&block.terminator)?;
        }

        Ok(())
    }

    pub(crate) fn ensure_local_alloca(
        &mut self,
        local: LocalId,
        ty: &AirType,
    ) -> Result<PointerValue<'static>, CodegenError> {
        if let Some(ptr) = self.local_allocas.get(&local).copied() {
            return Ok(ptr);
        }

        let alloca_ty = air_basic_type_to_llvm(ty, self.context)?;
        let ptr = self
            .builder
            .build_alloca(alloca_ty, &format!("l{}", local.0))
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;

        let align = alignment_for_type(self, ty)?;
        if let Some(inst) = ptr.as_instruction() {
            inst.set_alignment(align)
                .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        }

        self.local_allocas.insert(local, ptr);
        Ok(ptr)
    }

    fn entry_block(&self) -> Result<BasicBlock<'static>, CodegenError> {
        let first = self.air_function.blocks.first().ok_or_else(|| {
            CodegenError::UnsupportedInstruction(format!(
                "function `{}` has no blocks",
                self.air_function.name
            ))
        })?;
        self.lookup_block(first.id)
    }

    pub(crate) fn lookup_block(&self, id: BlockId) -> Result<BasicBlock<'static>, CodegenError> {
        self.block_map
            .get(&id)
            .copied()
            .ok_or_else(|| CodegenError::LlvmError(format!("unknown block {}", id.0)))
    }

    pub(crate) fn lookup_local_ptr(
        &self,
        local: LocalId,
    ) -> Result<PointerValue<'static>, CodegenError> {
        self.local_allocas
            .get(&local)
            .copied()
            .ok_or_else(|| CodegenError::LlvmError(format!("unknown local {}", local.0)))
    }

    pub(crate) fn local_air_type(&self, local: LocalId) -> Result<&AirType, CodegenError> {
        self.local_types
            .get(&local)
            .ok_or_else(|| CodegenError::UnsupportedType(format!("unknown local {}", local.0)))
    }

    pub(crate) fn load_local(
        &mut self,
        local: LocalId,
    ) -> Result<BasicValueEnum<'static>, CodegenError> {
        let ptr = self.lookup_local_ptr(local)?;
        let ty = air_basic_type_to_llvm(self.local_air_type(local)?, self.context)?;
        self.builder
            .build_load(ty, ptr, &format!("ld{}", local.0))
            .map_err(|e| CodegenError::LlvmError(e.to_string()))
    }
}

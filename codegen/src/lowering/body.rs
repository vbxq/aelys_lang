use crate::types::air_basic_type_to_llvm;
use crate::{AirNodeLocation, AirNodePosition, CodegenError};
use aelys_air::{
    AirFunction, AirProgram, AirStmtKind, AirType, BlockId, FunctionId, LocalId, Operand, Place,
    Rvalue,
};
use inkwell::basic_block::BasicBlock;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::values::{BasicValueEnum, FunctionValue, PointerValue};
use std::collections::{HashMap, HashSet};

pub(crate) struct FunctionCodegen<'a> {
    pub(crate) context: &'static Context,
    pub(crate) module: &'a Module<'static>,
    pub(crate) builder: Builder<'static>,
    pub(crate) function: FunctionValue<'static>,
    pub(crate) air_function: &'a AirFunction,
    pub(crate) program: &'a AirProgram,
    pub(crate) function_names: &'a HashMap<FunctionId, String>,
    pub(crate) block_map: HashMap<BlockId, BasicBlock<'static>>,
    pub(crate) alloca_locals: HashSet<LocalId>,
    pub(crate) alloca_map: HashMap<LocalId, PointerValue<'static>>,
    pub(crate) value_map: HashMap<LocalId, BasicValueEnum<'static>>,
    pub(crate) local_types: HashMap<LocalId, AirType>,
    pub(crate) string_id: u64,
    pub(crate) string_globals: HashMap<String, PointerValue<'static>>,
    pub(crate) current_block: Option<BlockId>,
    pub(crate) current_stmt_index: Option<usize>,
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
        let mut alloca_locals = HashSet::new();
        let param_ids: HashSet<LocalId> = air_function.params.iter().map(|p| p.id).collect();
        for param in &air_function.params {
            local_types.insert(param.id, param.ty.clone());
        }
        for local in &air_function.locals {
            local_types.insert(local.id, local.ty.clone());
            if !param_ids.contains(&local.id) && (local.name.is_some() || local.is_mut) {
                alloca_locals.insert(local.id);
            }
        }
        for block in &air_function.blocks {
            for stmt in &block.stmts {
                if let AirStmtKind::Assign { place, rvalue } = &stmt.kind {
                    if let Place::Field(local, _) = place {
                        alloca_locals.insert(*local);
                    }
                    if let Place::Index(local, _) = place {
                        alloca_locals.insert(*local);
                    }
                    if let Rvalue::AddressOf(local) = rvalue {
                        alloca_locals.insert(*local);
                    }
                    if let Rvalue::Index { base, .. } = rvalue {
                        if let Operand::Copy(local) | Operand::Move(local) = base {
                            if matches!(local_types.get(local), Some(AirType::Array(_, _))) {
                                alloca_locals.insert(*local);
                            }
                        }
                    }
                }
            }
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
            alloca_locals,
            alloca_map: HashMap::new(),
            value_map: HashMap::new(),
            local_types,
            string_id: 0,
            string_globals: HashMap::new(),
            current_block: None,
            current_stmt_index: None,
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

        let params = self.air_function.params.clone();
        for param in params {
            if self.local_uses_alloca(param.id) {
                self.ensure_local_alloca(param.id, &param.ty)?;
            }
        }

        let locals = self.air_function.locals.clone();
        for local in locals {
            if self.local_uses_alloca(local.id) {
                self.ensure_local_alloca(local.id, &local.ty)?;
            }
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
            if self.local_uses_alloca(param.id) {
                let ptr = self.lookup_local_ptr(param.id)?;
                self.store_value(ptr, value.into())?;
            } else {
                self.value_map.insert(param.id, value);
            }
        }

        Ok(())
    }

    fn generate_blocks(&mut self) -> Result<(), CodegenError> {
        let blocks = self.air_function.blocks.clone();
        for block in &blocks {
            self.current_block = Some(block.id);
            let llvm_block = self.lookup_block(block.id)?;
            self.builder.position_at_end(llvm_block);
            for (stmt_index, stmt) in block.stmts.iter().enumerate() {
                self.current_stmt_index = Some(stmt_index);
                self.generate_stmt(&stmt.kind)?;
            }
            self.current_stmt_index = None;
            self.generate_terminator(&block.terminator)?;
        }

        self.current_block = None;
        self.current_stmt_index = None;

        Ok(())
    }

    pub(crate) fn ensure_local_alloca(
        &mut self,
        local: LocalId,
        ty: &AirType,
    ) -> Result<PointerValue<'static>, CodegenError> {
        if let Some(ptr) = self.alloca_map.get(&local).copied() {
            return Ok(ptr);
        }
        if !self.local_uses_alloca(local) {
            return Err(CodegenError::LlvmError(format!(
                "local {} has no stack storage",
                local.0
            )));
        }

        let alloca_ty = air_basic_type_to_llvm(ty, self.context)?;
        let ptr = self
            .builder
            .build_alloca(alloca_ty, &format!("l{}", local.0))
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        self.align_alloca(ptr, alloca_ty)?;

        self.alloca_map.insert(local, ptr);
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
        self.alloca_map
            .get(&local)
            .copied()
            .ok_or_else(|| CodegenError::LlvmError(format!("unknown local {}", local.0)))
    }

    pub(crate) fn local_uses_alloca(&self, local: LocalId) -> bool {
        self.alloca_locals.contains(&local)
    }

    pub(crate) fn assign_local(
        &mut self,
        local: LocalId,
        value: BasicValueEnum<'static>,
    ) -> Result<(), CodegenError> {
        if let Some(ptr) = self.alloca_map.get(&local).copied() {
            return self.store_value(ptr, value);
        }
        if self.local_uses_alloca(local) {
            return Err(CodegenError::LlvmError(format!(
                "missing stack slot for local {}",
                local.0
            )));
        }
        self.value_map.insert(local, value);
        Ok(())
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
        if let Some(ptr) = self.alloca_map.get(&local).copied() {
            let ty = air_basic_type_to_llvm(self.local_air_type(local)?, self.context)?;
            return self.load_value(ty, ptr, &format!("ld{}", local.0));
        }
        self.value_map.get(&local).copied().ok_or_else(|| {
            CodegenError::LlvmError(format!("local {} used before assignment", local.0))
        })
    }

    pub(crate) fn unsupported_air(
        &self,
        kind: &'static str,
        detail: impl Into<String>,
    ) -> CodegenError {
        let position = match self.current_stmt_index {
            Some(index) => AirNodePosition::Stmt(index),
            None => AirNodePosition::Terminator,
        };
        let location = AirNodeLocation {
            function: self.air_function.name.clone(),
            block: self.current_block.map(|block| block.0),
            position,
        };
        CodegenError::unsupported_with_location(kind, detail, location)
    }
}

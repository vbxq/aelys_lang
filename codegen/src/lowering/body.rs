use crate::lowering::functions::{function_has_implicit_env, needs_sret};
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
    entry_block_id: BlockId,
    pub(crate) sret_ptr: Option<PointerValue<'static>>,
    interp_buffer: Option<PointerValue<'static>>,
}

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn target_is_windows(&self) -> bool {
        crate::module_targets_windows(self.module)
    }

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
        // and/or) would silently read stale values from codegen order instead
        let mut first_assign_block: HashMap<LocalId, BlockId> = HashMap::new();
        for block in &air_function.blocks {
            for stmt in &block.stmts {
                if let AirStmtKind::Assign { place, rvalue } = &stmt.kind {
                    if let Place::Local(local) = place {
                        if param_ids.contains(local) {
                            alloca_locals.insert(*local);
                        } else {
                            match first_assign_block.entry(*local) {
                                std::collections::hash_map::Entry::Vacant(e) => {
                                    e.insert(block.id);
                                }
                                std::collections::hash_map::Entry::Occupied(e) => {
                                    if *e.get() != block.id {
                                        alloca_locals.insert(*local);
                                    }
                                }
                            }
                        }
                    }
                    if let Place::Field(local, _) = place {
                        alloca_locals.insert(*local);
                    }
                    if let Place::Index(local, _) = place {
                        alloca_locals.insert(*local);
                    }
                    if let Rvalue::AddressOf(
                        Place::Local(local) | Place::Field(local, _) | Place::Index(local, _),
                    ) = rvalue
                    {
                        alloca_locals.insert(*local);
                    }
                    if let Rvalue::Len(Operand::Copy(local) | Operand::Move(local)) = rvalue {
                        if !matches!(local_types.get(local), Some(AirType::Ptr(_))) {
                            alloca_locals.insert(*local);
                        }
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

        let entry_block_id = find_entry_block(air_function);

        let is_windows = crate::module_targets_windows(module);
        let sret_ptr = if needs_sret(
            &air_function.ret_ty,
            air_function.calling_conv,
            is_windows,
            program,
        ) {
            Some(
                function
                    .get_nth_param(0)
                    .expect("invariant: sret function must have param 0")
                    .into_pointer_value(),
            )
        } else {
            None
        };

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
            entry_block_id,
            sret_ptr,
            interp_buffer: None,
        }
    }

    /// must match aelys_f64_str_buf in core/src/aelys_core_common.c, the callee writes blind
    pub(crate) const INTERP_BUFFER_BYTES: u32 = 64;

    /// one slot per frame, never per call site, or a print in a loop would grow the stack
    pub(crate) fn ensure_interp_buffer(&mut self) -> Result<PointerValue<'static>, CodegenError> {
        if let Some(ptr) = self.interp_buffer {
            return Ok(ptr);
        }
        let entry = self.entry_block()?;
        let resume = self.builder.get_insert_block();
        match entry.get_first_instruction() {
            Some(first) => self.builder.position_before(&first),
            None => self.builder.position_at_end(entry),
        }
        let buffer_ty = self.context.i8_type().array_type(Self::INTERP_BUFFER_BYTES);
        let ptr = self
            .builder
            .build_alloca(buffer_ty, "interp_buf")
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        self.align_alloca(ptr, buffer_ty.into())?;
        if let Some(block) = resume {
            self.builder.position_at_end(block);
        }
        self.interp_buffer = Some(ptr);
        Ok(ptr)
    }

    pub(crate) fn generate(&mut self) -> Result<(), CodegenError> {
        self.create_blocks();
        self.create_allocas()?;
        self.copy_params()?;
        self.generate_blocks()
    }

    fn ordered_block_ids(&self) -> Vec<BlockId> {
        let mut ids = Vec::with_capacity(self.air_function.blocks.len());
        ids.push(self.entry_block_id);
        for block in &self.air_function.blocks {
            if block.id != self.entry_block_id {
                ids.push(block.id);
            }
        }
        ids
    }

    fn create_blocks(&mut self) {
        for id in self.ordered_block_ids() {
            let name = format!("bb{}", id.0);
            let llvm_block = self.context.append_basic_block(self.function, &name);
            self.block_map.insert(id, llvm_block);
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
        let mut offset = if self.sret_ptr.is_some() { 1u32 } else { 0 };
        if function_has_implicit_env(self.air_function) {
            offset += 1; // skip implicit env param
        }
        for (index, param) in params.iter().enumerate() {
            let value = self
                .function
                .get_nth_param(index as u32 + offset)
                .ok_or_else(|| {
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
        let order = self.ordered_block_ids();

        for block_id in order {
            let block = blocks.iter().find(|b| b.id == block_id).unwrap();
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
        self.lookup_block(self.entry_block_id)
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

fn find_entry_block(func: &AirFunction) -> BlockId {
    use aelys_air::AirTerminator;

    let mut has_predecessors = HashSet::new();
    for block in &func.blocks {
        match &block.terminator {
            AirTerminator::Goto(target) => {
                has_predecessors.insert(*target);
            }
            AirTerminator::Branch {
                then_block,
                else_block,
                ..
            } => {
                has_predecessors.insert(*then_block);
                has_predecessors.insert(*else_block);
            }
            AirTerminator::Switch {
                targets, default, ..
            } => {
                for (_, target) in targets {
                    has_predecessors.insert(*target);
                }
                has_predecessors.insert(*default);
            }
            _ => {}
        }
    }

    for block in &func.blocks {
        if !has_predecessors.contains(&block.id) {
            return block.id;
        }
    }

    func.blocks.first().map(|b| b.id).unwrap_or(BlockId(0))
}

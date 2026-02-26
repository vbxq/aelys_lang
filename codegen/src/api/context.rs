use crate::CodegenError;
use aelys_air::AirProgram;
use inkwell::OptimizationLevel;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::targets::{
    CodeModel, FileType, InitializationConfig, RelocMode, Target, TargetMachine,
};
use std::path::Path;

pub struct CodegenContext {
    pub(crate) context: &'static Context,
    pub(crate) module: Module<'static>,
    pub(crate) builder: Builder<'static>,
}

impl CodegenContext {
    pub fn new(module_name: &str) -> Self {
        let context = Box::leak(Box::new(Context::create()));
        let module = context.create_module(module_name);
        let builder = context.create_builder();

        // Set target triple and data layout immediately so ABI decisions
        // (for eg sret, struct sizes/alignments) are correct during codegen
        Target::initialize_native(&InitializationConfig::default()).ok();
        let triple = TargetMachine::get_default_triple();
        module.set_triple(&triple);

        if let Ok(target) = Target::from_triple(&triple) {
            if let Some(machine) = target.create_target_machine(
                &triple,
                "generic",
                "",
                OptimizationLevel::None,
                RelocMode::Default,
                CodeModel::Default,
            ) {
                module.set_data_layout(&machine.get_target_data().get_data_layout());
            }
        }

        Self {
            context,
            module,
            builder,
        }
    }

    pub fn compile(&mut self, program: &AirProgram) -> Result<(), CodegenError> {
        let _ = self.builder.get_insert_block();
        self.declare_struct_types(program)?;
        self.declare_functions(program)?;
        self.define_function_bodies(program)?;
        self.emit_entry_wrapper(program)?;
        self.module
            .verify()
            .map_err(|e| CodegenError::LlvmError(e.to_string()))
    }

    pub fn emit_object(&self, path: &str) -> Result<(), CodegenError> {
        Target::initialize_native(&InitializationConfig::default())
            .map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        let triple = TargetMachine::get_default_triple();
        let target =
            Target::from_triple(&triple).map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        let cpu = TargetMachine::get_host_cpu_name().to_string();
        let features = TargetMachine::get_host_cpu_features().to_string();
        let target_machine = target
            .create_target_machine(
                &triple,
                &cpu,
                &features,
                OptimizationLevel::Aggressive,
                RelocMode::Default,
                CodeModel::Default,
            )
            .ok_or_else(|| {
                CodegenError::LlvmError("failed to create target machine".to_string())
            })?;

        target_machine
            .write_to_file(&self.module, FileType::Object, Path::new(path))
            .map_err(|e| CodegenError::LlvmError(e.to_string()))
    }

    pub fn emit_ir(&self, path: &str) -> Result<(), CodegenError> {
        self.module
            .print_to_file(path)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))
    }
}

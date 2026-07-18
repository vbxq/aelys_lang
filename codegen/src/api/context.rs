use crate::CodegenError;
use aelys_air::AirProgram;
use inkwell::OptimizationLevel;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;
use inkwell::passes::PassBuilderOptions;
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
        // a working native llvm backend is required, so this is fatal by design
        Target::initialize_native(&InitializationConfig::default())
            .expect("LLVM native target initialization failed");
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

    pub(crate) fn target_is_windows(&self) -> bool {
        crate::module_targets_windows(&self.module)
    }

    pub fn compile(&mut self, program: &AirProgram) -> Result<(), CodegenError> {
        let _ = self.builder.get_insert_block();
        self.declare_struct_types(program)?;
        self.declare_functions(program)?;
        self.declare_globals(program)?;
        self.emit_rc_type_table(program)?;
        self.define_function_bodies(program)?;
        self.emit_entry_wrapper(program)?;
        self.module
            .verify()
            .map_err(|e| CodegenError::LlvmError(e.to_string()))
    }

    pub fn optimize(&self, pass_pipeline: &str, opt_numeric: u8) -> Result<(), CodegenError> {
        let triple = TargetMachine::get_default_triple();
        let target =
            Target::from_triple(&triple).map_err(|e| CodegenError::LlvmError(e.to_string()))?;
        let cpu = TargetMachine::get_host_cpu_name().to_string();
        let features = TargetMachine::get_host_cpu_features().to_string();
        let machine = target
            .create_target_machine(
                &triple,
                &cpu,
                &features,
                inkwell_opt_level(opt_numeric),
                // PIC so it works on Linux
                RelocMode::PIC,
                CodeModel::Default,
            )
            .ok_or_else(|| {
                CodegenError::LlvmError("failed to create target machine".to_string())
            })?;

        self.module
            .run_passes(pass_pipeline, &machine, PassBuilderOptions::create())
            .map_err(|e| CodegenError::LlvmError(e.to_string()))
    }

    pub fn emit_object(&self, path: &str, opt_numeric: u8) -> Result<(), CodegenError> {
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
                inkwell_opt_level(opt_numeric),
                RelocMode::PIC,
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

fn inkwell_opt_level(numeric: u8) -> OptimizationLevel {
    match numeric {
        0 => OptimizationLevel::None,
        1 => OptimizationLevel::Less,
        3 => OptimizationLevel::Aggressive,
        _ => OptimizationLevel::Default,
    }
}

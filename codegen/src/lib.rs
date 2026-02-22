pub mod types;

mod body;
mod calls;
mod casts;
mod functions;
mod layout;
mod operands;
mod ops;
mod rvalues;
mod runtime;
mod stmts;
mod structs;
mod terminators;

use aelys_air::AirProgram;
use inkwell::builder::Builder;
use inkwell::context::Context;
use inkwell::module::Module;

pub struct CodegenContext {
    context: &'static Context,
    module: Module<'static>,
    builder: Builder<'static>,
}

impl CodegenContext {
    pub fn new(module_name: &str) -> Self {
        let context = Box::leak(Box::new(Context::create()));
        let module = context.create_module(module_name);
        let builder = context.create_builder();
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
        self.module
            .verify()
            .map_err(|e| CodegenError::LlvmError(e.to_string()))
    }

    pub fn emit_object(&self, path: &str) -> Result<(), CodegenError> {
        let _ = path;
        todo!()
    }

    pub fn emit_ir(&self, path: &str) -> Result<(), CodegenError> {
        self.module
            .print_to_file(path)
            .map_err(|e| CodegenError::LlvmError(e.to_string()))
    }
}

#[derive(Debug)]
pub enum CodegenError {
    LlvmError(String),
    UnsupportedType(String),
    UnsupportedInstruction(String),
}

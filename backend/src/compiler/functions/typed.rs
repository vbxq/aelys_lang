use super::super::Compiler;
use super::typed_body::compile_typed_body;
use super::typed_finalize::finalize_typed_function;
use super::typed_setup::setup_typed_function;
use aelys_common::Result;

impl Compiler {
    pub fn compile_typed_function(&mut self, func: &aelys_sema::TypedFunction) -> Result<()> {
        let setup = setup_typed_function(self, func)?;
        let mut nested_compiler = setup.nested_compiler;

        compile_typed_body(&mut nested_compiler, func, setup.has_no_gc)?;

        finalize_typed_function(self, nested_compiler, func, setup.func_var_reg)
    }
}

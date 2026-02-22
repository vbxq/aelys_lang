use crate::CodegenError;
use crate::body::FunctionCodegen;
use inkwell::AddressSpace;
use inkwell::values::{FunctionValue, PointerValue};

impl<'a> FunctionCodegen<'a> {
    pub(crate) fn ensure_alloc_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_alloc") {
            return function;
        }

        let fn_ty = self
            .context
            .ptr_type(AddressSpace::default())
            .fn_type(&[self.context.i64_type().into()], false);
        self.module.add_function("__aelys_alloc", fn_ty, None)
    }

    pub(crate) fn ensure_free_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_free") {
            return function;
        }

        let fn_ty = self.context.void_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("__aelys_free", fn_ty, None)
    }

    pub(crate) fn ensure_panic_function(&self) -> FunctionValue<'static> {
        if let Some(function) = self.module.get_function("__aelys_panic") {
            return function;
        }

        let fn_ty = self.context.void_type().fn_type(
            &[self.context.ptr_type(AddressSpace::default()).into()],
            false,
        );
        self.module.add_function("__aelys_panic", fn_ty, None)
    }

    pub(crate) fn global_string_ptr(&mut self, text: &str) -> Result<PointerValue<'static>, CodegenError> {
        let name = format!("str_{}_{}", self.air_function.id.0, self.string_id);
        self.string_id = self.string_id.saturating_add(1);
        self.builder
            .build_global_string_ptr(text, &name)
            .map(|g| g.as_pointer_value())
            .map_err(|e| CodegenError::LlvmError(e.to_string()))
    }
}

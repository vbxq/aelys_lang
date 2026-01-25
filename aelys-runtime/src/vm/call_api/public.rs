use super::super::{VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

impl VM {
    /// Call a function by name with the given arguments.
    pub fn call_function_by_name(
        &mut self,
        name: &str,
        args: &[Value],
    ) -> Result<Value, RuntimeError> {
        let func_value = self.globals.get(name).copied().ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::UndefinedVariable(format!(
                "function '{}' not found",
                name
            )))
        })?;

        self.call_value(func_value, args)
    }

    /// Get a function value by name for repeated calls.
    pub fn get_function_value(&self, name: &str) -> Option<Value> {
        self.globals.get(name).copied()
    }
}

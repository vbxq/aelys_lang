use super::VM;
use super::Value;
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

mod numbers;
mod strings;

impl VM {
    /// Add two values (integers, floats, or strings).
    /// Handles mixed int/float by promoting to float.
    pub fn add_values(&mut self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        if let Some(result) = numbers::try_add_numbers(left, right) {
            return Ok(result);
        }

        if let Some(result) = self.try_concat_strings(left, right)? {
            return Ok(result);
        }

        Err(self.runtime_error(RuntimeErrorKind::TypeError {
            operation: "addition",
            expected: "number or string",
            got: format!(
                "{} and {}",
                self.value_type_name(left),
                self.value_type_name(right)
            ),
        }))
    }

    /// Subtract two values (integers or floats).
    pub fn sub_values(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        if let Some(result) = numbers::try_sub_numbers(left, right) {
            return Ok(result);
        }

        Err(self.runtime_error(RuntimeErrorKind::TypeError {
            operation: "subtraction",
            expected: "number",
            got: format!(
                "{} and {}",
                self.value_type_name(left),
                self.value_type_name(right)
            ),
        }))
    }

    /// Multiply two values (integers or floats).
    pub fn mul_values(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        if let Some(result) = numbers::try_mul_numbers(left, right) {
            return Ok(result);
        }

        Err(self.runtime_error(RuntimeErrorKind::TypeError {
            operation: "multiplication",
            expected: "number",
            got: format!(
                "{} and {}",
                self.value_type_name(left),
                self.value_type_name(right)
            ),
        }))
    }

    /// Divide two values (integers or floats).
    /// Integer division truncates toward zero.
    pub fn div_values(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        if let Some(result) = numbers::try_div_numbers(self, left, right)? {
            return Ok(result);
        }

        Err(self.runtime_error(RuntimeErrorKind::TypeError {
            operation: "division",
            expected: "number",
            got: format!(
                "{} and {}",
                self.value_type_name(left),
                self.value_type_name(right)
            ),
        }))
    }

    /// Modulo two values (integers or floats).
    pub fn mod_values(&self, left: Value, right: Value) -> Result<Value, RuntimeError> {
        if let Some(result) = numbers::try_mod_numbers(self, left, right)? {
            return Ok(result);
        }

        Err(self.runtime_error(RuntimeErrorKind::TypeError {
            operation: "modulo",
            expected: "number",
            got: format!(
                "{} and {}",
                self.value_type_name(left),
                self.value_type_name(right)
            ),
        }))
    }

    /// Negate a value (unary minus).
    pub fn neg_value(&self, operand: Value) -> Result<Value, RuntimeError> {
        if let Some(n) = operand.as_int() {
            return Ok(Value::int(-n));
        }
        if let Some(f) = operand.as_float() {
            return Ok(Value::float(-f));
        }

        Err(self.runtime_error(RuntimeErrorKind::TypeError {
            operation: "negation",
            expected: "number",
            got: self.value_type_name(operand).to_string(),
        }))
    }
}

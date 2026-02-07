use super::Value;

impl Value {
    /// Check if value is truthy.
    pub fn is_truthy(&self) -> bool {
        if self.is_null() {
            false
        } else if let Some(b) = self.as_bool() {
            b
        } else if let Some(n) = self.as_int() {
            n != 0
        } else if let Some(n) = self.as_float() {
            n != 0.0
        } else {
            true
        }
    }

    /// Get type name for error messages.
    pub fn type_name(&self) -> &'static str {
        if self.is_null() {
            "Null"
        } else if self.is_bool() {
            "Bool"
        } else if self.is_int() {
            "Int"
        } else if self.is_float() {
            "Float"
        } else if self.is_ptr() {
            "Object"
        } else {
            "Unknown"
        }
    }
}

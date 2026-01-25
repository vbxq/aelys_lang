use super::TypeEnv;
use crate::types::InferType;

impl TypeEnv {
    /// Define a captured variable (upvalue)
    pub fn define_capture(&mut self, name: String, ty: InferType) {
        self.captures.insert(name, ty);
    }

    /// Get all captures
    pub fn captures(&self) -> &std::collections::HashMap<String, InferType> {
        &self.captures
    }
}

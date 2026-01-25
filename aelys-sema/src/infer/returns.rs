use super::TypeInference;
use crate::types::InferType;

impl TypeInference {
    /// Push expected return type when entering a function
    pub(super) fn push_return_type(&mut self, ty: InferType) {
        self.return_type_stack.push(ty);
    }

    /// Pop return type when exiting a function
    pub(super) fn pop_return_type(&mut self) -> Option<InferType> {
        self.return_type_stack.pop()
    }

    /// Get current expected return type
    pub(super) fn current_return_type(&self) -> Option<&InferType> {
        self.return_type_stack.last()
    }
}

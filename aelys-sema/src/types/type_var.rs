use std::fmt;

/// Unique identifier for type variables during inference
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeVarId(pub u32);

impl fmt::Display for TypeVarId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "τ{}", self.0)
    }
}

/// Type variable generator - produces fresh type variables
#[derive(Debug, Default)]
pub struct TypeVarGen {
    next: u32,
}

impl TypeVarGen {
    pub fn new() -> Self {
        Self { next: 0 }
    }

    /// Generate a fresh type variable
    pub fn fresh(&mut self) -> super::InferType {
        let id = TypeVarId(self.next);
        self.next += 1;
        super::InferType::Var(id)
    }

    /// Generate a fresh type variable ID only
    pub fn fresh_id(&mut self) -> TypeVarId {
        let id = TypeVarId(self.next);
        self.next += 1;
        id
    }

    /// Get the current count (for debugging)
    pub fn count(&self) -> u32 {
        self.next
    }
}

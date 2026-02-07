use crate::Function;

/// A wrapped bytecode function for the GC heap.
#[derive(Debug, Clone)]
pub struct AelysFunction {
    pub function: Function,
    pub verified: bool,
}

impl AelysFunction {
    pub fn new(function: Function) -> Self {
        Self {
            function,
            verified: false,
        }
    }

    pub fn name(&self) -> Option<&str> {
        self.function.name.as_deref()
    }

    pub fn arity(&self) -> u8 {
        self.function.arity
    }

    pub fn num_registers(&self) -> u8 {
        self.function.num_registers
    }
}

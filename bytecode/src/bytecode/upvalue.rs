/// Describes how to capture an upvalue for a closure.
#[derive(Debug, Clone, Copy)]
pub struct UpvalueDescriptor {
    /// If true, capture from enclosing function's locals (register).
    /// If false, capture from enclosing function's upvalues.
    pub is_local: bool,
    /// The index: register number if is_local, upvalue index otherwise.
    pub index: u8,
}

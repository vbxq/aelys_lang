use super::GcRef;

/// Cached metadata for faster closure execution
#[derive(Debug, Clone)]
pub struct ClosureCache {
    pub bytecode_ptr: *const u32,
    pub bytecode_len: usize,
    pub constants_ptr: *const crate::value::Value,
    pub constants_len: usize,
    pub arity: u8,
    pub num_registers: u8,
}

/// A closure wraps a function with its captured upvalues.
#[derive(Debug, Clone)]
pub struct AelysClosure {
    pub function: GcRef,
    pub upvalues: Vec<GcRef>,
    pub bytecode_ptr: *const u32,
    pub bytecode_len: usize,
    pub constants_ptr: *const crate::value::Value,
    pub constants_len: usize,
    pub arity: u8,
    pub num_registers: u8,
}

impl AelysClosure {
    pub fn new(function: GcRef, upvalues: Vec<GcRef>) -> Self {
        Self {
            function,
            upvalues,
            bytecode_ptr: std::ptr::null(),
            bytecode_len: 0,
            constants_ptr: std::ptr::null(),
            constants_len: 0,
            arity: 0,
            num_registers: 0,
        }
    }

    pub fn with_cache(function: GcRef, upvalues: Vec<GcRef>, cache: ClosureCache) -> Self {
        Self {
            function,
            upvalues,
            bytecode_ptr: cache.bytecode_ptr,
            bytecode_len: cache.bytecode_len,
            constants_ptr: cache.constants_ptr,
            constants_len: cache.constants_len,
            arity: cache.arity,
            num_registers: cache.num_registers,
        }
    }
}

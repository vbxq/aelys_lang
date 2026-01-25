use super::GcRef;

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

    pub fn with_cache(
        function: GcRef,
        upvalues: Vec<GcRef>,
        bytecode_ptr: *const u32,
        bytecode_len: usize,
        constants_ptr: *const crate::value::Value,
        constants_len: usize,
        arity: u8,
        num_registers: u8,
    ) -> Self {
        Self {
            function,
            upvalues,
            bytecode_ptr,
            bytecode_len,
            constants_ptr,
            constants_len,
            arity,
            num_registers,
        }
    }
}

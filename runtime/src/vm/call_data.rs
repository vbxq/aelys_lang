use super::{GcRef, NativeFunction, Value};

/// Cached function call data to avoid repeated heap lookups.
pub(super) enum CallData {
    Function {
        func_ref: GcRef,
        arity: u8,
        num_registers: u8,
        bytecode_ptr: *const u32,
        bytecode_len: usize,
        constants_ptr: *const Value,
        constants_len: usize,
    },
    Native {
        native: NativeFunction,
    },
    Closure {
        inner_func_ref: GcRef,
        arity: u8,
        num_registers: u8,
        bytecode_ptr: *const u32,
        bytecode_len: usize,
        constants_ptr: *const Value,
        constants_len: usize,
        upvalues_ptr: *const GcRef,
        upvalues_len: usize,
    },
}

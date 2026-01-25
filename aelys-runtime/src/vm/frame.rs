use super::{GcRef, Value};

// The windowed register approach is borrowed from Lua's implementation.
// Each function sees r0, r1, r2... but they're actually offsets from `base`
// in a shared register stack. This avoids copying arguments on calls.
//
// Note: all the raw pointers here are cached to avoid going through the heap
// on every instruction. The safety invariants are documented on each field.

#[derive(Debug, Clone)]
pub struct CallFrame {
    pub function: GcRef,
    pub ip: usize,
    pub base: usize,           // offset into shared register stack
    pub return_dest: u8,
    pub bytecode_ptr: *const u32,   // cached for speed
    pub bytecode_len: usize,
    pub constants_ptr: *const Value,
    pub constants_len: usize,
    pub upvalues_ptr: *const GcRef, // null for non-closures
    pub upvalues_len: usize,
    pub num_registers: u8,
    pub global_mapping_id: usize,
}

impl CallFrame {
    pub fn new(
        function: GcRef,
        base: usize,
        bytecode_ptr: *const u32,
        bytecode_len: usize,
        constants_ptr: *const Value,
        constants_len: usize,
        num_registers: u8,
    ) -> Self {
        Self {
            function,
            ip: 0,
            base,
            return_dest: 0,
            bytecode_ptr,
            bytecode_len,
            constants_ptr,
            constants_len,
            upvalues_ptr: std::ptr::null(),
            upvalues_len: 0,
            num_registers,
            global_mapping_id: 0,
        }
    }

    #[inline(always)]
    pub fn with_return_dest(
        function: GcRef,
        base: usize,
        return_dest: u8,
        bytecode_ptr: *const u32,
        bytecode_len: usize,
        constants_ptr: *const Value,
        constants_len: usize,
        num_registers: u8,
    ) -> Self {
        Self {
            function,
            ip: 0,
            base,
            return_dest,
            bytecode_ptr,
            bytecode_len,
            constants_ptr,
            constants_len,
            upvalues_ptr: std::ptr::null(),
            upvalues_len: 0,
            num_registers,
            global_mapping_id: 0,
        }
    }

    #[inline(always)]
    pub fn with_upvalues(
        function: GcRef,
        base: usize,
        return_dest: u8,
        bytecode_ptr: *const u32,
        bytecode_len: usize,
        constants_ptr: *const Value,
        constants_len: usize,
        upvalues_ptr: *const GcRef,
        upvalues_len: usize,
        num_registers: u8,
    ) -> Self {
        Self {
            function,
            ip: 0,
            base,
            return_dest,
            bytecode_ptr,
            bytecode_len,
            constants_ptr,
            constants_len,
            upvalues_ptr,
            upvalues_len,
            num_registers,
            global_mapping_id: 0,
        }
    }

    pub fn return_dest(&self) -> u8 { self.return_dest }
    pub fn function(&self) -> GcRef { self.function }
    pub fn ip(&self) -> usize { self.ip }
    pub fn base(&self) -> usize { self.base }
    pub fn advance_ip(&mut self) { self.ip += 1; }
    pub fn set_ip(&mut self, ip: usize) { self.ip = ip; }

    pub fn jump(&mut self, offset: i16) {
        if offset >= 0 {
            self.ip += offset as usize;
        } else {
            self.ip = self.ip.saturating_sub((-offset) as usize);
        }
    }

    pub fn register_index(&self, reg: u8) -> Option<usize> {
        self.base.checked_add(reg as usize)
    }
}

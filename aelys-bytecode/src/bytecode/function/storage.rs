use super::Function;
use crate::bytecode::{BytecodeBuffer, OpCode};

impl Function {
    /// Set bytecode directly from a Vec (for assembler/binary loading).
    pub fn set_bytecode(&mut self, bytecode: Vec<u32>) {
        self.bytecode = BytecodeBuffer::from_vec(bytecode);
        self.bytecode_builder.clear();
    }

    /// Push a raw instruction to the bytecode builder.
    pub fn push_raw(&mut self, instr: u32) {
        self.bytecode_builder.push(instr);
    }

    /// Get a mutable reference to a bytecode instruction at the given index.
    pub fn bytecode_mut(&mut self, index: usize) -> &mut u32 {
        &mut self.bytecode_builder[index]
    }

    /// Get an immutable reference to a bytecode instruction at the given index.
    pub fn bytecode_at(&self, index: usize) -> u32 {
        self.bytecode_builder[index]
    }

    /// Get current instruction count (for jump patching)
    pub fn current_offset(&self) -> usize {
        self.bytecode_builder.len()
    }

    /// Emit a jump and return its offset for later patching
    pub fn emit_jump(&mut self, op: OpCode, line: u32) -> usize {
        let offset = self.current_offset();
        self.emit_b(op, 0, 0, line);
        offset
    }

    /// Emit a conditional jump and return its offset
    pub fn emit_jump_if(&mut self, op: OpCode, reg: u8, line: u32) -> usize {
        let offset = self.current_offset();
        self.emit_b(op, reg, 0, line);
        offset
    }
}

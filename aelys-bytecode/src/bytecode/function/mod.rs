use super::buffer::BytecodeBuffer;
use super::global_layout::GlobalLayout;
use super::opcode::OpCode;
use super::upvalue::UpvalueDescriptor;
use crate::value::Value;
use std::sync::Arc;

mod constants;
mod lines;
mod patch;
mod registers;
mod storage;

// A compiled function. BytecodeBuffer allows patching for inline caches
// while keeping raw pointers stable (important for dispatch loop perf).
#[derive(Debug, Clone)]
pub struct Function {
    pub name: Option<String>,
    pub arity: u8,
    pub num_registers: u8,
    pub call_site_count: u16, // for MIC pre-allocation
    pub bytecode: BytecodeBuffer,
    bytecode_builder: Vec<u32>, // temp storage during compilation
    pub constants: Vec<Value>,
    pub nested_functions: Vec<Function>,
    pub upvalue_descriptors: Vec<UpvalueDescriptor>,
    pub lines: Vec<(u16, u32)>,
    pub global_layout: Arc<GlobalLayout>,
    pub global_layout_hash: u64,
}

impl Function {
    pub fn new(name: Option<String>, arity: u8) -> Self {
        Self {
            name,
            arity,
            num_registers: 0,
            call_site_count: 0,
            bytecode: BytecodeBuffer::empty(),
            bytecode_builder: Vec::new(),
            constants: Vec::new(),
            nested_functions: Vec::new(),
            upvalue_descriptors: Vec::new(),
            lines: Vec::new(),
            global_layout: GlobalLayout::empty(),
            global_layout_hash: 0,
        }
    }

    pub fn finalize_bytecode(&mut self) {
        if !self.bytecode_builder.is_empty() {
            self.bytecode = BytecodeBuffer::from_vec(std::mem::take(&mut self.bytecode_builder));
        }
        // make sure we have enough registers for the bytecode
        let needed = registers::required_registers(self.bytecode.as_slice());
        if needed > self.num_registers as usize {
            self.num_registers = needed.min(255) as u8;
        }
        for f in &mut self.nested_functions { f.finalize_bytecode(); }
    }

    // format A: op|a|b|c (3 regs)
    pub fn emit_a(&mut self, op: OpCode, a: u8, b: u8, c: u8, line: u32) {
        self.emit_raw(((op as u32) << 24) | ((a as u32) << 16) | ((b as u32) << 8) | c as u32, line);
    }

    // format B: op|a|imm16
    pub fn emit_b(&mut self, op: OpCode, a: u8, imm: i16, line: u32) {
        self.emit_raw(((op as u32) << 24) | ((a as u32) << 16) | (imm as u16) as u32, line);
    }

    // format C: same layout as A but semantics are dest|func|nargs
    pub fn emit_c(&mut self, op: OpCode, dest: u8, func: u8, nargs: u8, line: u32) {
        self.emit_a(op, dest, func, nargs, line);
    }

    fn emit_raw(&mut self, instr: u32, line: u32) {
        self.bytecode_builder.push(instr);
        self.add_line(line);
    }
}

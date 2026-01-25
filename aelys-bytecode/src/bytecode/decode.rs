use super::opcode::OpCode;

// instruction formats:
//   A: op(8) | a(8) | b(8) | c(8)   - 3 regs
//   B: op(8) | a(8) | imm(16)       - reg + signed immediate
//   C: same as A                    - just different semantics (call)

pub fn decode_a(instr: u32) -> (OpCode, u8, u8, u8) {
    let op = OpCode::from_u8((instr >> 24) as u8).unwrap_or(OpCode::Move);
    (op, ((instr >> 16) & 0xFF) as u8, ((instr >> 8) & 0xFF) as u8, (instr & 0xFF) as u8)
}

pub fn decode_b(instr: u32) -> (OpCode, u8, i16) {
    let op = OpCode::from_u8((instr >> 24) as u8).unwrap_or(OpCode::Move);
    (op, ((instr >> 16) & 0xFF) as u8, (instr & 0xFFFF) as i16)
}

pub fn decode_c(instr: u32) -> (OpCode, u8, u8, u8) { decode_a(instr) }

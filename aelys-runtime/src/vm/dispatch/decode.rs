/// Decode ABC format instruction.
#[inline(always)]
pub(super) fn decode_abc(instr: u32) -> (u8, u8, u8) {
    let a = ((instr >> 16) & 0xFF) as u8;
    let b = ((instr >> 8) & 0xFF) as u8;
    let c = (instr & 0xFF) as u8;
    (a, b, c)
}

/// Decode A + imm16 format instruction.
#[inline(always)]
pub(super) fn decode_aimm(instr: u32) -> (u8, i16) {
    let a = ((instr >> 16) & 0xFF) as u8;
    let imm = (instr & 0xFFFF) as i16;
    (a, imm)
}

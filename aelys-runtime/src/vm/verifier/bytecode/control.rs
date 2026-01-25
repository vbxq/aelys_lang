use crate::vm::OpCode;

use super::{verify_jump, verify_reg};

pub(super) fn verify(
    opcode: OpCode,
    ip: usize,
    a: usize,
    _b: usize,
    _c: usize,
    imm: i16,
    num_regs: usize,
    bytecode_len: usize,
) -> Result<bool, String> {
    match opcode {
        OpCode::Jump => {
            verify_jump(ip, imm, bytecode_len, "Jump")?;
        }
        OpCode::JumpIf | OpCode::JumpIfNot => {
            verify_reg(a, num_regs, "JumpIf")?;
            verify_jump(ip, imm, bytecode_len, "JumpIf")?;
        }
        OpCode::Return => {
            verify_reg(a, num_regs, "Return")?;
        }
        OpCode::Return0 => {}
        OpCode::EnterNoGc | OpCode::ExitNoGc => {}
        OpCode::ForLoopI | OpCode::ForLoopIInc => {
            verify_reg(a, num_regs, "ForLoopI")?;
            verify_reg(a + 1, num_regs, "ForLoopI")?;
            verify_reg(a + 2, num_regs, "ForLoopI")?;
            verify_jump(ip, imm, bytecode_len, "ForLoopI")?;
        }
        OpCode::WhileLoopLt => {
            verify_reg(a, num_regs, "WhileLoopLt")?;
            verify_reg(a + 1, num_regs, "WhileLoopLt")?;
            verify_jump(ip, imm, bytecode_len, "WhileLoopLt")?;
        }
        _ => return Ok(false),
    }

    Ok(true)
}

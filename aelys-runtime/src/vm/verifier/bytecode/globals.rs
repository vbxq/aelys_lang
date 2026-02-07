use crate::vm::OpCode;

use super::{verify_call_args, verify_const, verify_reg};

#[allow(clippy::too_many_arguments)]
pub(super) fn verify(
    opcode: OpCode,
    ip: usize,
    a: usize,
    b: usize,
    c: usize,
    _imm: i16,
    num_regs: usize,
    constants_len: usize,
    bytecode_len: usize,
) -> Result<bool, String> {
    match opcode {
        OpCode::GetGlobal | OpCode::SetGlobal => {
            verify_reg(a, num_regs, "Global")?;
            verify_const(b, constants_len, "Global")?;
        }
        OpCode::IncGlobalI => {
            verify_reg(a, num_regs, "IncGlobalI")?;
            verify_const(b, constants_len, "IncGlobalI")?;
        }
        OpCode::GetGlobalIdx | OpCode::SetGlobalIdx => {
            verify_reg(a, num_regs, "GlobalIdx")?;
        }
        OpCode::CallGlobal | OpCode::CallGlobalMono | OpCode::CallGlobalNative => {
            verify_reg(a, num_regs, "CallGlobal")?;
            verify_call_args(a, c, num_regs, "CallGlobal")?;
            // security: CallGlobal opcodes require 2 cache words after the instruction
            // validate that ip + 3 <= bytecode_len (1 instruction + 2 cache words)
            if ip + 3 > bytecode_len {
                return Err(format!(
                    "CallGlobal at ip {} requires 2 cache words but bytecode length is {} (need at least {})",
                    ip,
                    bytecode_len,
                    ip + 3
                ));
            }
        }
        _ => return Ok(false),
    }

    Ok(true)
}

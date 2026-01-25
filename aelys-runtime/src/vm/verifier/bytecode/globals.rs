use crate::vm::OpCode;

use super::{verify_call_args, verify_const, verify_reg};

pub(super) fn verify(
    opcode: OpCode,
    a: usize,
    b: usize,
    c: usize,
    _imm: i16,
    num_regs: usize,
    constants_len: usize,
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
        }
        _ => return Ok(false),
    }

    Ok(true)
}

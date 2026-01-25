use crate::vm::OpCode;

use super::{verify_call_args, verify_reg, verify_upval};

pub(super) fn verify(
    opcode: OpCode,
    a: usize,
    b: usize,
    c: usize,
    num_regs: usize,
    upvalues_len: usize,
) -> Result<bool, String> {
    match opcode {
        OpCode::Call => {
            verify_reg(a, num_regs, "Call")?;
            verify_reg(b, num_regs, "Call")?;
            verify_call_args(b, c, num_regs, "Call")?;
        }
        OpCode::CallCached => {
            verify_reg(a, num_regs, "CallCached")?;
            verify_reg(b, num_regs, "CallCached")?;
            verify_call_args(b, c, num_regs, "CallCached")?;
        }
        OpCode::CallUpval | OpCode::TailCallUpval => {
            verify_reg(a, num_regs, "CallUpval")?;
            verify_upval(b, upvalues_len, "CallUpval")?;
            verify_call_args(a, c, num_regs, "CallUpval")?;
        }
        _ => return Ok(false),
    }

    Ok(true)
}

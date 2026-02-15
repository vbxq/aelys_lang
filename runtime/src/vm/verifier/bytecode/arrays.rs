use crate::vm::OpCode;

use super::{verify_reg, verify_reg_range};

pub(super) fn verify(
    opcode: OpCode,
    a: usize,
    b: usize,
    c: usize,
    num_regs: usize,
) -> Result<bool, String> {
    match opcode {
        OpCode::ArrayNewI | OpCode::ArrayNewF | OpCode::ArrayNewB | OpCode::ArrayNewP => {
            verify_reg(a, num_regs, "ArrayNew")?;
            verify_reg(b, num_regs, "ArrayNew")?;
        }
        OpCode::ArrayLit => {
            verify_reg(a, num_regs, "ArrayLit")?;
            verify_reg_range(b, c, num_regs, "ArrayLit")?;
        }
        OpCode::ArrayLoadI | OpCode::ArrayLoadF | OpCode::ArrayLoadB | OpCode::ArrayLoadP => {
            verify_reg(a, num_regs, "ArrayLoad")?;
            verify_reg(b, num_regs, "ArrayLoad")?;
            verify_reg(c, num_regs, "ArrayLoad")?;
        }
        OpCode::ArrayGetI | OpCode::ArrayGetF | OpCode::ArrayGetB | OpCode::ArrayGetP => {
            verify_reg(a, num_regs, "ArrayGet")?;
            verify_reg(b, num_regs, "ArrayGet")?;
            verify_reg(c, num_regs, "ArrayGet")?;
        }
        OpCode::ArrayStoreI | OpCode::ArrayStoreF | OpCode::ArrayStoreB | OpCode::ArrayStoreP => {
            verify_reg(a, num_regs, "ArrayStore")?;
            verify_reg(b, num_regs, "ArrayStore")?;
            verify_reg(c, num_regs, "ArrayStore")?;
        }
        OpCode::ArrayLen => {
            verify_reg(a, num_regs, "ArrayLen")?;
            verify_reg(b, num_regs, "ArrayLen")?;
        }
        OpCode::VecNewI | OpCode::VecNewF | OpCode::VecNewB | OpCode::VecNewP => {
            verify_reg(a, num_regs, "VecNew")?;
        }
        OpCode::VecLit => {
            verify_reg(a, num_regs, "VecLit")?;
            verify_reg_range(b, c, num_regs, "VecLit")?;
        }
        OpCode::VecPushI | OpCode::VecPushF | OpCode::VecPushB | OpCode::VecPushP => {
            verify_reg(a, num_regs, "VecPush")?;
            verify_reg(b, num_regs, "VecPush")?;
        }
        OpCode::VecPopI | OpCode::VecPopF | OpCode::VecPopB | OpCode::VecPopP => {
            verify_reg(a, num_regs, "VecPop")?;
            verify_reg(b, num_regs, "VecPop")?;
        }
        OpCode::VecLen | OpCode::VecCap => {
            verify_reg(a, num_regs, "VecLen/Cap")?;
            verify_reg(b, num_regs, "VecLen/Cap")?;
        }
        OpCode::VecReserve => {
            verify_reg(a, num_regs, "VecReserve")?;
            verify_reg(b, num_regs, "VecReserve")?;
        }
        OpCode::VecLoadI | OpCode::VecLoadF | OpCode::VecLoadB | OpCode::VecLoadP => {
            verify_reg(a, num_regs, "VecLoad")?;
            verify_reg(b, num_regs, "VecLoad")?;
            verify_reg(c, num_regs, "VecLoad")?;
        }
        OpCode::VecGetI | OpCode::VecGetF | OpCode::VecGetB | OpCode::VecGetP => {
            verify_reg(a, num_regs, "VecGet")?;
            verify_reg(b, num_regs, "VecGet")?;
            verify_reg(c, num_regs, "VecGet")?;
        }
        OpCode::VecStoreI | OpCode::VecStoreF | OpCode::VecStoreB | OpCode::VecStoreP => {
            verify_reg(a, num_regs, "VecStore")?;
            verify_reg(b, num_regs, "VecStore")?;
            verify_reg(c, num_regs, "VecStore")?;
        }
        OpCode::StringLoadChar => {
            verify_reg(a, num_regs, "StringLoadChar")?;
            verify_reg(b, num_regs, "StringLoadChar")?;
            verify_reg(c, num_regs, "StringLoadChar")?;
        }

        _ => return Ok(false),
    }

    Ok(true)
}

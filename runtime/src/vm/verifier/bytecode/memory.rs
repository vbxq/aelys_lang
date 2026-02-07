use crate::vm::OpCode;

use super::verify_reg;

pub(super) fn verify(
    opcode: OpCode,
    a: usize,
    b: usize,
    c: usize,
    num_regs: usize,
) -> Result<bool, String> {
    match opcode {
        OpCode::Alloc => {
            verify_reg(a, num_regs, "Alloc")?;
            verify_reg(b, num_regs, "Alloc")?;
        }
        OpCode::Free => {
            verify_reg(a, num_regs, "Free")?;
        }
        OpCode::LoadMem => {
            verify_reg(a, num_regs, "LoadMem")?;
            verify_reg(b, num_regs, "LoadMem")?;
            verify_reg(c, num_regs, "LoadMem")?;
        }
        OpCode::LoadMemI => {
            verify_reg(a, num_regs, "LoadMemI")?;
            verify_reg(b, num_regs, "LoadMemI")?;
        }
        OpCode::StoreMem => {
            verify_reg(a, num_regs, "StoreMem")?;
            verify_reg(b, num_regs, "StoreMem")?;
            verify_reg(c, num_regs, "StoreMem")?;
        }
        OpCode::StoreMemI => {
            verify_reg(a, num_regs, "StoreMemI")?;
            verify_reg(c, num_regs, "StoreMemI")?;
        }
        _ => return Ok(false),
    }

    Ok(true)
}

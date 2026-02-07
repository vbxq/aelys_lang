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
        OpCode::Add
        | OpCode::Sub
        | OpCode::Mul
        | OpCode::Div
        | OpCode::Mod
        | OpCode::Eq
        | OpCode::Ne
        | OpCode::Lt
        | OpCode::Le
        | OpCode::Gt
        | OpCode::Ge
        | OpCode::AddII
        | OpCode::SubII
        | OpCode::MulII
        | OpCode::DivII
        | OpCode::ModII
        | OpCode::AddFF
        | OpCode::SubFF
        | OpCode::MulFF
        | OpCode::DivFF
        | OpCode::ModFF
        | OpCode::LtII
        | OpCode::LeII
        | OpCode::GtII
        | OpCode::GeII
        | OpCode::EqII
        | OpCode::NeII
        | OpCode::LtFF
        | OpCode::LeFF
        | OpCode::GtFF
        | OpCode::GeFF
        | OpCode::EqFF
        | OpCode::NeFF
        | OpCode::AddIIG
        | OpCode::SubIIG
        | OpCode::MulIIG
        | OpCode::DivIIG
        | OpCode::ModIIG
        | OpCode::AddFFG
        | OpCode::SubFFG
        | OpCode::MulFFG
        | OpCode::DivFFG
        | OpCode::ModFFG
        | OpCode::LtIIG
        | OpCode::LeIIG
        | OpCode::GtIIG
        | OpCode::GeIIG
        | OpCode::EqIIG
        | OpCode::NeIIG
        | OpCode::LtFFG
        | OpCode::LeFFG
        | OpCode::GtFFG
        | OpCode::GeFFG
        | OpCode::EqFFG
        | OpCode::NeFFG
        | OpCode::Shl
        | OpCode::Shr
        | OpCode::BitAnd
        | OpCode::BitOr
        | OpCode::BitXor
        | OpCode::ShlII
        | OpCode::ShrII
        | OpCode::AndII
        | OpCode::OrII
        | OpCode::XorII => {
            verify_reg(a, num_regs, "BinOp")?;
            verify_reg(b, num_regs, "BinOp")?;
            verify_reg(c, num_regs, "BinOp")?;
        }
        OpCode::ShlIImm | OpCode::ShrIImm | OpCode::AndIImm | OpCode::OrIImm | OpCode::XorIImm => {
            verify_reg(a, num_regs, "BitIImm")?;
            verify_reg(b, num_regs, "BitIImm")?;
        }
        OpCode::Neg | OpCode::Not | OpCode::BitNot | OpCode::NotI => {
            verify_reg(a, num_regs, "UnaryOp")?;
            verify_reg(b, num_regs, "UnaryOp")?;
        }
        OpCode::AddI | OpCode::SubI => {
            verify_reg(a, num_regs, "AddI")?;
            verify_reg(b, num_regs, "AddI")?;
        }
        OpCode::LtImm | OpCode::LeImm | OpCode::GtImm | OpCode::GeImm => {
            verify_reg(a, num_regs, "CmpImm")?;
            verify_reg(b, num_regs, "CmpImm")?;
        }
        OpCode::LtIImm | OpCode::LeIImm | OpCode::GtIImm | OpCode::GeIImm => {
            verify_reg(a, num_regs, "CmpIImm")?;
            verify_reg(b, num_regs, "CmpIImm")?;
        }
        _ => return Ok(false),
    }

    Ok(true)
}

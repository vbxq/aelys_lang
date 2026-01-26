use crate::bytecode::OpCode;
use crate::bytecode::decode_a;

pub(super) fn required_registers(bytecode: &[u32]) -> usize {
    let mut max_reg: usize = 0;
    let mut used = false;
    let mut ip = 0;

    while ip < bytecode.len() {
        let instr = bytecode[ip];
        let (op, a, b, c) = decode_a(instr);
        let imm = (instr & 0xFFFF) as i16;

        match op {
            OpCode::Move => {
                update_max_reg(&mut max_reg, &mut used, a as usize, Some(b as usize), None)
            }
            OpCode::LoadI | OpCode::LoadNull | OpCode::LoadBool | OpCode::LoadK => {
                update_max_reg(&mut max_reg, &mut used, a as usize, None, None);
            }
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
                update_max_reg(
                    &mut max_reg,
                    &mut used,
                    a as usize,
                    Some(b as usize),
                    Some(c as usize),
                );
            }
            OpCode::ShlIImm
            | OpCode::ShrIImm
            | OpCode::AndIImm
            | OpCode::OrIImm
            | OpCode::XorIImm => {
                update_max_reg(&mut max_reg, &mut used, a as usize, Some(b as usize), None);
            }
            OpCode::Neg | OpCode::Not | OpCode::BitNot | OpCode::NotI => {
                update_max_reg(&mut max_reg, &mut used, a as usize, Some(b as usize), None);
            }
            OpCode::Jump => {
                let _ = imm;
            }
            OpCode::JumpIf | OpCode::JumpIfNot => {
                update_max_reg(&mut max_reg, &mut used, a as usize, None, None);
            }
            OpCode::Call => {
                let nargs = c as usize;
                update_max_reg(&mut max_reg, &mut used, a as usize, Some(b as usize), None);
                if nargs > 0 {
                    update_max_reg(&mut max_reg, &mut used, (b as usize) + nargs, None, None);
                }
            }
            OpCode::Return => {
                update_max_reg(&mut max_reg, &mut used, a as usize, None, None);
            }
            OpCode::Return0 => {}
            OpCode::GetGlobal | OpCode::SetGlobal | OpCode::IncGlobalI => {
                update_max_reg(&mut max_reg, &mut used, a as usize, None, None);
            }
            OpCode::EnterNoGc | OpCode::ExitNoGc => {}
            OpCode::Alloc => {
                update_max_reg(&mut max_reg, &mut used, a as usize, Some(b as usize), None);
            }
            OpCode::Free => {
                update_max_reg(&mut max_reg, &mut used, a as usize, None, None);
            }
            OpCode::LoadMem => {
                update_max_reg(
                    &mut max_reg,
                    &mut used,
                    a as usize,
                    Some(b as usize),
                    Some(c as usize),
                );
            }
            OpCode::LoadMemI => {
                update_max_reg(&mut max_reg, &mut used, a as usize, Some(b as usize), None);
            }
            OpCode::StoreMem => {
                update_max_reg(
                    &mut max_reg,
                    &mut used,
                    a as usize,
                    Some(b as usize),
                    Some(c as usize),
                );
            }
            OpCode::StoreMemI => {
                update_max_reg(&mut max_reg, &mut used, a as usize, Some(c as usize), None);
            }
            OpCode::Print => {
                update_max_reg(&mut max_reg, &mut used, a as usize, None, None);
            }
            OpCode::MakeClosure | OpCode::GetUpval | OpCode::CloseUpvals => {
                update_max_reg(&mut max_reg, &mut used, a as usize, None, None);
            }
            OpCode::SetUpval => {
                update_max_reg(&mut max_reg, &mut used, b as usize, None, None);
            }
            OpCode::ForLoopI | OpCode::ForLoopIInc => {
                update_max_reg(
                    &mut max_reg,
                    &mut used,
                    a as usize,
                    Some((a as usize) + 1),
                    Some((a as usize) + 2),
                );
            }
            OpCode::AddI | OpCode::SubI => {
                update_max_reg(&mut max_reg, &mut used, a as usize, Some(b as usize), None);
            }
            OpCode::LtImm | OpCode::LeImm | OpCode::GtImm | OpCode::GeImm => {
                update_max_reg(&mut max_reg, &mut used, a as usize, Some(b as usize), None);
            }
            OpCode::WhileLoopLt => {
                update_max_reg(
                    &mut max_reg,
                    &mut used,
                    a as usize,
                    Some((a as usize) + 1),
                    None,
                );
            }
            OpCode::LtIImm | OpCode::LeIImm | OpCode::GtIImm | OpCode::GeIImm => {
                update_max_reg(&mut max_reg, &mut used, a as usize, Some(b as usize), None);
            }
            OpCode::GetGlobalIdx | OpCode::SetGlobalIdx => {
                update_max_reg(&mut max_reg, &mut used, a as usize, None, None);
            }
            OpCode::CallGlobal | OpCode::CallGlobalMono | OpCode::CallGlobalNative => {
                let nargs = c as usize;
                update_max_reg(&mut max_reg, &mut used, a as usize, None, None);
                if nargs > 0 {
                    update_max_reg(&mut max_reg, &mut used, (a as usize) + nargs, None, None);
                }
                ip += 2; // skip cache words
            }
            OpCode::CallCached => {
                let nargs = c as usize;
                update_max_reg(&mut max_reg, &mut used, a as usize, Some(b as usize), None);
                if nargs > 0 {
                    update_max_reg(&mut max_reg, &mut used, (b as usize) + nargs, None, None);
                }
            }
            OpCode::CallUpval | OpCode::TailCallUpval => {
                let nargs = c as usize;
                update_max_reg(&mut max_reg, &mut used, a as usize, None, None);
                if nargs > 0 {
                    update_max_reg(&mut max_reg, &mut used, (a as usize) + nargs, None, None);
                }
            }
        }
        ip += 1;
    }

    if used { max_reg + 1 } else { 0 }
}

fn update_max_reg(
    max_reg: &mut usize,
    used: &mut bool,
    a: usize,
    b: Option<usize>,
    c: Option<usize>,
) {
    *used = true;
    *max_reg = (*max_reg).max(a);
    if let Some(b) = b {
        *max_reg = (*max_reg).max(b);
    }
    if let Some(c) = c {
        *max_reg = (*max_reg).max(c);
    }
}

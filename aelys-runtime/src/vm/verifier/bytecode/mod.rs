use crate::vm::{Function, OpCode};

mod arithmetic;
mod calls;
mod closures;
mod control;
mod globals;
mod memory;
mod registers;

use super::checks::{
    check_call_args, check_const_index, check_jump, check_reg, check_reg_range, check_upval_index,
};

pub(super) fn verify_bytecode(func: &Function) -> Result<(), String> {
    let num_regs = func.num_registers as usize;
    let constants_len = func.constants.len();
    let upvalues_len = func.upvalue_descriptors.len();
    let bytecode = &func.bytecode;

    // security: Validate function size limits to prevent integer truncation
    // when caching bytecode_len as u32 and constants_len as u16 in CallSiteCacheEntry
    if bytecode.len() > u32::MAX as usize {
        return Err(format!(
            "bytecode length {} exceeds maximum {} (u32::MAX)",
            bytecode.len(),
            u32::MAX
        ));
    }
    if constants_len > u16::MAX as usize {
        return Err(format!(
            "constants length {} exceeds maximum {} (u16::MAX)",
            constants_len,
            u16::MAX
        ));
    }

    let mut ip = 0;
    while ip < bytecode.len() {
        let instr = bytecode[ip];
        let opcode_byte = (instr >> 24) as u8;
        let opcode = OpCode::from_u8(opcode_byte)
            .ok_or_else(|| format!("invalid opcode {} at {}", opcode_byte, ip))?;

        let a = ((instr >> 16) & 0xFF) as usize;
        let b = ((instr >> 8) & 0xFF) as usize;
        let c = (instr & 0xFF) as usize;
        let imm = (instr & 0xFFFF) as i16;

        // skip cache words after CallGlobal variants
        let skip = matches!(
            opcode,
            OpCode::CallGlobal | OpCode::CallGlobalMono | OpCode::CallGlobalNative
        );

        if registers::verify(opcode, a, b, c, imm, num_regs, constants_len)? {
            ip += if skip { 3 } else { 1 };
            continue;
        }
        if arithmetic::verify(opcode, a, b, c, num_regs)? {
            ip += 1;
            continue;
        }
        if control::verify(opcode, ip, a, b, c, imm, num_regs, bytecode.len())? {
            ip += 1;
            continue;
        }
        if memory::verify(opcode, a, b, c, num_regs)? {
            ip += 1;
            continue;
        }
        if globals::verify(opcode, ip, a, b, c, imm, num_regs, constants_len, bytecode.len())? {
            ip += if skip { 3 } else { 1 };
            continue;
        }
        if calls::verify(opcode, a, b, c, num_regs, upvalues_len)? {
            ip += 1;
            continue;
        }
        if closures::verify(func, opcode, a, b, c, num_regs, constants_len, upvalues_len)? {
            ip += 1;
            continue;
        }

        return Err(format!("unhandled opcode {:?} at {}", opcode, ip));
    }

    Ok(())
}

pub(super) fn verify_call_args(
    base_reg: usize,
    nargs: usize,
    num_regs: usize,
    op: &str,
) -> Result<(), String> {
    check_reg(base_reg, num_regs, op)?;
    check_call_args(base_reg, nargs, num_regs, op)
}

pub(super) fn verify_const(idx: usize, constants_len: usize, op: &str) -> Result<(), String> {
    check_const_index(idx, constants_len, op)
}

pub(super) fn verify_jump(
    ip: usize,
    imm: i16,
    bytecode_len: usize,
    op: &str,
) -> Result<(), String> {
    check_jump(ip, imm, bytecode_len, op)
}

pub(super) fn verify_reg(reg: usize, num_regs: usize, op: &str) -> Result<(), String> {
    check_reg(reg, num_regs, op)
}

pub(super) fn verify_upval(idx: usize, upvalues_len: usize, op: &str) -> Result<(), String> {
    check_upval_index(idx, upvalues_len, op)
}

pub(super) fn verify_reg_range(
    base: usize,
    count: usize,
    num_regs: usize,
    op: &str,
) -> Result<(), String> {
    check_reg_range(base, count, num_regs, op)
}

pub(super) fn check_reg(reg: usize, num_regs: usize, op: &str) -> Result<(), String> {
    if reg >= num_regs {
        return Err(format!(
            "{} uses register {} out of bounds (max {})",
            op,
            reg,
            num_regs.saturating_sub(1)
        ));
    }
    Ok(())
}

/// validates a range of consecutive registers starting at `base` with `count` registers.
/// uses checked arithmetic to prevent overflow attacks.
pub(super) fn check_reg_range(
    base: usize,
    count: usize,
    num_regs: usize,
    op: &str,
) -> Result<(), String> {
    if count == 0 {
        return Ok(());
    }
    // Check that base + count - 1 doesn't overflow and is within bounds
    let last = base
        .checked_add(count - 1)
        .ok_or_else(|| format!("{} register range overflow at base {}", op, base))?;
    if last >= num_regs {
        return Err(format!(
            "{} uses registers r{}..r{} but only {} registers available",
            op, base, last, num_regs
        ));
    }
    Ok(())
}

pub(super) fn check_const_index(idx: usize, constants_len: usize, op: &str) -> Result<(), String> {
    if idx >= constants_len {
        return Err(format!(
            "{} uses constant {} out of bounds (max {})",
            op,
            idx,
            constants_len.saturating_sub(1)
        ));
    }
    Ok(())
}

pub(super) fn check_upval_index(idx: usize, upvalues_len: usize, op: &str) -> Result<(), String> {
    if idx >= upvalues_len {
        return Err(format!(
            "{} uses upvalue {} out of bounds (max {})",
            op,
            idx,
            upvalues_len.saturating_sub(1)
        ));
    }
    Ok(())
}

pub(super) fn check_call_args(
    start_reg: usize,
    nargs: usize,
    num_regs: usize,
    op: &str,
) -> Result<(), String> {
    if nargs == 0 {
        return Ok(());
    }
    let last = start_reg
        .checked_add(nargs)
        .ok_or_else(|| format!("{} argument overflow", op))?;
    if last >= num_regs {
        return Err(format!(
            "{} uses args through r{} out of bounds (max {})",
            op,
            last,
            num_regs.saturating_sub(1)
        ));
    }
    Ok(())
}

pub(super) fn check_jump(ip: usize, offset: i16, bc_len: usize, op: &str) -> Result<(), String> {
    let next_ip = ip
        .checked_add(1)
        .ok_or_else(|| format!("{} ip overflow", op))?;
    let target = (next_ip as isize)
        .checked_add(offset as isize)
        .ok_or_else(|| format!("{} jump overflow", op))?;
    if target < 0 || target as usize > bc_len {
        return Err(format!("{} jump target {} out of bounds", op, target));
    }
    Ok(())
}

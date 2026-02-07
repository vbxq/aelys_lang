// Control flow operations: Not(17), Jump(18), JumpIf(19), JumpIfNot(20),
// ForLoopI(40), ForLoopIInc(41), LtImm(44)-GeImm(47), WhileLoopLt(48)

match opcode_byte {
    // Not (17)
    17 => {
        let (a, b, _) = decode_abc(instr);
        let value = reg_get!(base + b as usize);
        let is_falsy = value.is_null()
            || value.as_bool() == Some(false)
            || value.as_int() == Some(0);
        reg_set!(base + a as usize, Value::bool(is_falsy));
    }

    // Jump (18)
    18 => {
        let (_, imm) = decode_aimm(instr);
        ip = (ip as isize + imm as isize) as usize;
    }

    // JumpIf (19)
    19 => {
        let (a, imm) = decode_aimm(instr);
        let value = reg_get!(base + a as usize);
        // Truthy: not null and not false
        if !value.is_null() && value.as_bool() != Some(false) {
            ip = (ip as isize + imm as isize) as usize;
        }
    }

    // JumpIfNot (20)
    20 => {
        let (a, imm) = decode_aimm(instr);
        let value = reg_get!(base + a as usize);
        // Falsy: null or false
        if value.is_null() || value.as_bool() == Some(false) {
            ip = (ip as isize + imm as isize) as usize;
        }
    }

    // ForLoopI (40)
    40 => {
        let (a, imm) = decode_aimm(instr);
        let iter_idx = base + a as usize;

        let iter = reg_ref!(iter_idx).as_int_unchecked();
        let end = reg_ref!(iter_idx + 1).as_int_unchecked();
        let step = reg_ref!(iter_idx + 2).as_int_unchecked();

        let new_iter = iter.wrapping_add(step);
        reg_set!(iter_idx, Value::int(new_iter));

        let should_continue = if step > 0 {
            new_iter < end
        } else {
            new_iter > end
        };
        if should_continue {
            ip = (ip as isize + imm as isize) as usize;
        }
    }

    // ForLoopIInc (41)
    41 => {
        let (a, imm) = decode_aimm(instr);
        let iter_idx = base + a as usize;

        let iter = reg_ref!(iter_idx).as_int_unchecked();
        let end = reg_ref!(iter_idx + 1).as_int_unchecked();
        let step = reg_ref!(iter_idx + 2).as_int_unchecked();

        let new_iter = iter.wrapping_add(step);
        reg_set!(iter_idx, Value::int(new_iter));

        let should_continue = if step > 0 {
            new_iter <= end
        } else {
            new_iter >= end
        };
        if should_continue {
            ip = (ip as isize + imm as isize) as usize;
        }
    }

    // LtImm (44)
    44 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l < c as i64));
    }

    // LeImm (45)
    45 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l <= c as i64));
    }

    // GtImm (46)
    46 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l > c as i64));
    }

    // GeImm (47)
    47 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l >= c as i64));
    }

    // WhileLoopLt (48)
    48 => {
        let (a, imm) = decode_aimm(instr);
        let iter = reg_ref!(base + a as usize).as_int_unchecked();
        let limit = reg_ref!(base + a as usize + 1).as_int_unchecked();
        if iter < limit {
            ip = (ip as isize + imm as isize) as usize;
        }
    }

    _ => unreachable!(),
}

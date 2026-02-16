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

    // StringForLoop (177)
    177 => {
        let (a, imm) = decode_aimm(instr);
        let char_idx = base + a as usize;
        let offset_idx = char_idx + 1;
        let str_idx = char_idx + 2;

        let str_val = reg_get!(str_idx);
        let byte_offset = reg_get!(offset_idx).as_int().unwrap_or(0) as usize;

        let str_ref = GcRef::new(str_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(str_ref)
            && let ObjectKind::String(s) = &obj.kind
        {
            let s_str = s.as_str();
            if byte_offset < s_str.len() {
                let remaining = &s_str[byte_offset..];
                if let Some(ch) = remaining.chars().next() {
                    let char_len = ch.len_utf8();
                    let mut buf = [0u8; 4];
                    let char_str = ch.encode_utf8(&mut buf);
                    self.frames[current_frame_idx].ip = ip;
                    match self.intern_string(char_str) {
                        Ok(new_ref) => {
                            reg_set!(char_idx, Value::ptr(new_ref.index()));
                            reg_set!(offset_idx, Value::int((byte_offset + char_len) as i64));
                            ip = (ip as isize + imm as isize) as usize;
                        }
                        Err(e) => return Err(e),
                    }
                }
                // else: invalid UTF-8, just exit loop
            }
            // else: byte_offset >= len, loop ends (don't jump)
        }
    }

    // VecForLoop (178)
    178 => {
        let (a, imm) = decode_aimm(instr);
        let elem_idx = base + a as usize;
        let index_idx = elem_idx + 1;
        let vec_idx = elem_idx + 2;

        let vec_val = reg_get!(vec_idx);
        let index = reg_get!(index_idx).as_int().unwrap_or(0);

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(vec_ref)
            && let ObjectKind::Vec(v) = &obj.kind
            && (index as usize) < v.len()
            && let Some(elem) = v.get(index as usize)
        {
            reg_set!(elem_idx, elem);
            reg_set!(index_idx, Value::int(index + 1));
            ip = (ip as isize + imm as isize) as usize;
        }
    }

    // ArrayForLoop (179)
    179 => {
        let (a, imm) = decode_aimm(instr);
        let elem_idx = base + a as usize;
        let index_idx = elem_idx + 1;
        let arr_idx = elem_idx + 2;

        let arr_val = reg_get!(arr_idx);
        let index = reg_get!(index_idx).as_int().unwrap_or(0);

        let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(arr_ref)
            && let ObjectKind::Array(a) = &obj.kind
            && (index as usize) < a.len()
            && let Some(elem) = a.get(index as usize)
        {
            reg_set!(elem_idx, elem);
            reg_set!(index_idx, Value::int(index + 1));
            ip = (ip as isize + imm as isize) as usize;
        }
    }

    _ => unreachable!(),
}

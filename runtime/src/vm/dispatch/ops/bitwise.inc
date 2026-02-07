// Bitwise operations: Shl(105), Shr(106), BitAnd(107), BitOr(108), BitXor(109),
// BitNot(110), ShlII(111)-XorII(115), NotI(116), ShlIImm(117)-XorIImm(121)

match opcode_byte {
    // Shl (105) - Generic left shift
    105 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);

        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            // Mask shift amount to 0-63 range (branchless, prevents Rust UB)
            reg_set!(base + a as usize, Value::int(l << (r & 63)));
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "<<",
                expected: "integer",
                got: format!("{} and {}", self.value_type_name(left), self.value_type_name(right)),
            }));
        }
    }

    // Shr (106) - Generic arithmetic right shift
    106 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);

        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            // Mask shift amount to 0-63 range (branchless, prevents Rust UB)
            reg_set!(base + a as usize, Value::int(l >> (r & 63)));
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                operation: ">>",
                expected: "integer",
                got: format!("{} and {}", self.value_type_name(left), self.value_type_name(right)),
            }));
        }
    }

    // BitAnd (107) - Generic bitwise AND
    107 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);

        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::int(l & r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "&",
                expected: "integer",
                got: format!("{} and {}", self.value_type_name(left), self.value_type_name(right)),
            }));
        }
    }

    // BitOr (108) - Generic bitwise OR
    108 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);

        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::int(l | r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "|",
                expected: "integer",
                got: format!("{} and {}", self.value_type_name(left), self.value_type_name(right)),
            }));
        }
    }

    // BitXor (109) - Generic bitwise XOR
    109 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);

        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::int(l ^ r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "^",
                expected: "integer",
                got: format!("{} and {}", self.value_type_name(left), self.value_type_name(right)),
            }));
        }
    }

    // BitNot (110) - Generic bitwise NOT
    110 => {
        let (a, b, _) = decode_abc(instr);
        let value = reg_get!(base + b as usize);

        if let Some(n) = value.as_int() {
            reg_set!(base + a as usize, Value::int(!n));
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "~",
                expected: "integer",
                got: self.value_type_name(value).to_string(),
            }));
        }
    }

    // ShlII (111) - Type-specialized left shift
    111 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        // Mask shift amount to 0-63 range (branchless, prevents Rust UB)
        reg_set!(base + a as usize, Value::int(l << (r & 63)));
    }

    // ShrII (112) - Type-specialized right shift
    112 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        // Mask shift amount to 0-63 range (branchless, prevents Rust UB)
        reg_set!(base + a as usize, Value::int(l >> (r & 63)));
    }

    // AndII (113) - Type-specialized bitwise AND
    113 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(l & r));
    }

    // OrII (114) - Type-specialized bitwise OR
    114 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(l | r));
    }

    // XorII (115) - Type-specialized bitwise XOR
    115 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(l ^ r));
    }

    // NotI (116) - Type-specialized bitwise NOT
    116 => {
        let (a, b, _) = decode_abc(instr);
        let n = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(!n));
    }

    // ShlIImm (117) - Left shift with immediate
    117 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        // Mask shift amount to 0-63 range (branchless, prevents Rust UB)
        reg_set!(base + a as usize, Value::int(l << ((c & 63) as i64)));
    }

    // ShrIImm (118) - Right shift with immediate
    118 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        // Mask shift amount to 0-63 range (branchless, prevents Rust UB)
        reg_set!(base + a as usize, Value::int(l >> ((c & 63) as i64)));
    }

    // AndIImm (119) - Bitwise AND with immediate
    119 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(l & (c as i64)));
    }

    // OrIImm (120) - Bitwise OR with immediate
    120 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(l | (c as i64)));
    }

    // XorIImm (121) - Bitwise XOR with immediate
    121 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(l ^ (c as i64)));
    }

    _ => unreachable!(),
}

// Arithmetic: Add(5), Sub(6), Mul(7), Div(8), Mod(9), Neg(10),
// plus the specialized int/float variants (AddI, AddII, AddFF, etc)

match opcode_byte {
    // Add (5)
    5 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);

        // int+int is common enough to deserve a fast path
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::int(l.wrapping_add(r)));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.add_values(left, right) {
                Ok(result) => reg_set!(base + a as usize, result),
                Err(e) => return Err(e),
            }
        }
    }

    // Sub (6)
    6 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);

        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::int(l.wrapping_sub(r)));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.sub_values(left, right) {
                Ok(result) => reg_set!(base + a as usize, result),
                Err(e) => return Err(e),
            }
        }
    }

    // Mul (7)
    7 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);

        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::int(l.wrapping_mul(r)));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.mul_values(left, right) {
                Ok(result) => reg_set!(base + a as usize, result),
                Err(e) => return Err(e),
            }
        }
    }

    // Div (8) - no fast path because we need to check for div by zero anyway
    8 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        self.frames[current_frame_idx].ip = ip;
        match self.div_values(left, right) {
            Ok(result) => reg_set!(base + a as usize, result),
            Err(e) => return Err(e),
        }
    }

    // Mod (9) - same deal
    9 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        self.frames[current_frame_idx].ip = ip;
        match self.mod_values(left, right) {
            Ok(result) => reg_set!(base + a as usize, result),
            Err(e) => return Err(e),
        }
    }

    // Neg (10)
    10 => {
        let (a, b, _) = decode_abc(instr);
        let value = reg_get!(base + b as usize);
        if let Some(n) = value.as_int() {
            reg_set!(base + a as usize, Value::int(-n));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.neg_value(value) {
                Ok(result) => reg_set!(base + a as usize, result),
                Err(e) => return Err(e),
            }
        }
    }

    // AddI (42)
    42 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(l.wrapping_add(c as i64)));
    }

    // SubI (43)
    43 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(l.wrapping_sub(c as i64)));
    }

    // AddII (49)
    49 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(l.wrapping_add(r)));
    }

    // SubII (50)
    50 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(l.wrapping_sub(r)));
    }

    // MulII (51)
    51 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::int(l.wrapping_mul(r)));
    }

    // DivII (52)
    52 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        if r == 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::DivisionByZero));
        }
        reg_set!(base + a as usize, Value::int(l / r));
    }

    // ModII (53)
    53 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        if r == 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::DivisionByZero));
        }
        reg_set!(base + a as usize, Value::int(l % r));
    }

    // AddFF (54)
    54 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_float_unchecked();
        let r = reg_ref!(base + c as usize).as_float_unchecked();
        reg_set!(base + a as usize, Value::float(l + r));
    }

    // SubFF (55)
    55 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_float_unchecked();
        let r = reg_ref!(base + c as usize).as_float_unchecked();
        reg_set!(base + a as usize, Value::float(l - r));
    }

    // MulFF (56)
    56 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_float_unchecked();
        let r = reg_ref!(base + c as usize).as_float_unchecked();
        reg_set!(base + a as usize, Value::float(l * r));
    }

    // DivFF (57)
    57 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_float_unchecked();
        let r = reg_ref!(base + c as usize).as_float_unchecked();
        reg_set!(base + a as usize, Value::float(l / r));
    }

    // ModFF (58)
    58 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_float_unchecked();
        let r = reg_ref!(base + c as usize).as_float_unchecked();
        reg_set!(base + a as usize, Value::float(l % r));
    }

    // AddIIG (82)
    82 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::int(l.wrapping_add(r)));
        } else {
            let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
            let r_float = right
                .as_float()
                .or_else(|| right.as_int().map(|i| i as f64));
            if let (Some(l), Some(r)) = (l_float, r_float) {
                reg_set!(base + a as usize, Value::float(l + r));
            } else {
                self.frames[current_frame_idx].ip = ip;
                match self.add_values(left, right) {
                    Ok(result) => reg_set!(base + a as usize, result),
                    Err(e) => return Err(e),
                }
            }
        }
    }

    // SubIIG (83)
    83 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::int(l.wrapping_sub(r)));
        } else {
            let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
            let r_float = right
                .as_float()
                .or_else(|| right.as_int().map(|i| i as f64));
            if let (Some(l), Some(r)) = (l_float, r_float) {
                reg_set!(base + a as usize, Value::float(l - r));
            } else {
                self.frames[current_frame_idx].ip = ip;
                match self.sub_values(left, right) {
                    Ok(result) => reg_set!(base + a as usize, result),
                    Err(e) => return Err(e),
                }
            }
        }
    }

    // MulIIG (84)
    84 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::int(l.wrapping_mul(r)));
        } else {
            let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
            let r_float = right
                .as_float()
                .or_else(|| right.as_int().map(|i| i as f64));
            if let (Some(l), Some(r)) = (l_float, r_float) {
                reg_set!(base + a as usize, Value::float(l * r));
            } else {
                self.frames[current_frame_idx].ip = ip;
                match self.mul_values(left, right) {
                    Ok(result) => reg_set!(base + a as usize, result),
                    Err(e) => return Err(e),
                }
            }
        }
    }

    // DivIIG (85)
    85 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            if r == 0 {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::DivisionByZero));
            }
            reg_set!(base + a as usize, Value::int(l / r));
        } else {
            let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
            let r_float = right
                .as_float()
                .or_else(|| right.as_int().map(|i| i as f64));
            if let (Some(l), Some(r)) = (l_float, r_float) {
                reg_set!(base + a as usize, Value::float(l / r));
            } else {
                self.frames[current_frame_idx].ip = ip;
                match self.div_values(left, right) {
                    Ok(result) => reg_set!(base + a as usize, result),
                    Err(e) => return Err(e),
                }
            }
        }
    }

    // ModIIG (86)
    86 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            if r == 0 {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::DivisionByZero));
            }
            reg_set!(base + a as usize, Value::int(l % r));
        } else {
            let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
            let r_float = right
                .as_float()
                .or_else(|| right.as_int().map(|i| i as f64));
            if let (Some(l), Some(r)) = (l_float, r_float) {
                reg_set!(base + a as usize, Value::float(l % r));
            } else {
                self.frames[current_frame_idx].ip = ip;
                match self.mod_values(left, right) {
                    Ok(result) => reg_set!(base + a as usize, result),
                    Err(e) => return Err(e),
                }
            }
        }
    }

    // AddFFG (87)
    87 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
        let r_float = right
            .as_float()
            .or_else(|| right.as_int().map(|i| i as f64));
        if let (Some(l), Some(r)) = (l_float, r_float) {
            reg_set!(base + a as usize, Value::float(l + r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.add_values(left, right) {
                Ok(result) => reg_set!(base + a as usize, result),
                Err(e) => return Err(e),
            }
        }
    }

    // SubFFG (88)
    88 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
        let r_float = right
            .as_float()
            .or_else(|| right.as_int().map(|i| i as f64));
        if let (Some(l), Some(r)) = (l_float, r_float) {
            reg_set!(base + a as usize, Value::float(l - r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.sub_values(left, right) {
                Ok(result) => reg_set!(base + a as usize, result),
                Err(e) => return Err(e),
            }
        }
    }

    // MulFFG (89)
    89 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
        let r_float = right
            .as_float()
            .or_else(|| right.as_int().map(|i| i as f64));
        if let (Some(l), Some(r)) = (l_float, r_float) {
            reg_set!(base + a as usize, Value::float(l * r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.mul_values(left, right) {
                Ok(result) => reg_set!(base + a as usize, result),
                Err(e) => return Err(e),
            }
        }
    }

    // DivFFG (90)
    90 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
        let r_float = right
            .as_float()
            .or_else(|| right.as_int().map(|i| i as f64));
        if let (Some(l), Some(r)) = (l_float, r_float) {
            reg_set!(base + a as usize, Value::float(l / r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.div_values(left, right) {
                Ok(result) => reg_set!(base + a as usize, result),
                Err(e) => return Err(e),
            }
        }
    }

    // ModFFG (91)
    91 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
        let r_float = right
            .as_float()
            .or_else(|| right.as_int().map(|i| i as f64));
        if let (Some(l), Some(r)) = (l_float, r_float) {
            reg_set!(base + a as usize, Value::float(l % r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.mod_values(left, right) {
                Ok(result) => reg_set!(base + a as usize, result),
                Err(e) => return Err(e),
            }
        }
    }

    _ => unreachable!(),
}

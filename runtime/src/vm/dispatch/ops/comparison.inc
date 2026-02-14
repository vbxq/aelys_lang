// comparisons (11-16) + specialized variants

match opcode_byte {
    // Eq/Ne - with string content comparison for heap objects
    11 => {
        let (a, b, c) = decode_abc(instr);
        let lhs = reg_get!(base + b as usize);
        let rhs = reg_get!(base + c as usize);
        let result = if lhs == rhs {
            true
        } else if let (Some(lp), Some(rp)) = (lhs.as_ptr(), rhs.as_ptr()) {
            let l_ref = GcRef::new(lp);
            let r_ref = GcRef::new(rp);
            match (self.heap.get(l_ref), self.heap.get(r_ref)) {
                (Some(lo), Some(ro)) => match (&lo.kind, &ro.kind) {
                    (ObjectKind::String(ls), ObjectKind::String(rs)) => ls == rs,
                    _ => false,
                },
                _ => false,
            }
        } else {
            false
        };
        reg_set!(base + a as usize, Value::bool(result));
    }
    12 => {
        let (a, b, c) = decode_abc(instr);
        let lhs = reg_get!(base + b as usize);
        let rhs = reg_get!(base + c as usize);
        let result = if lhs == rhs {
            true
        } else if let (Some(lp), Some(rp)) = (lhs.as_ptr(), rhs.as_ptr()) {
            let l_ref = GcRef::new(lp);
            let r_ref = GcRef::new(rp);
            match (self.heap.get(l_ref), self.heap.get(r_ref)) {
                (Some(lo), Some(ro)) => match (&lo.kind, &ro.kind) {
                    (ObjectKind::String(ls), ObjectKind::String(rs)) => ls == rs,
                    _ => false,
                },
                _ => false,
            }
        } else {
            false
        };
        reg_set!(base + a as usize, Value::bool(!result));
    }

    // Lt - int fastpath
    13 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::bool(l < r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.compare_lt(left, right) {
                Ok(res) => reg_set!(base + a as usize, Value::bool(res)),
                Err(e) => return Err(e),
            }
        }
    }

    // Le (14)
    14 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::bool(l <= r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.compare_le(left, right) {
                Ok(result) => reg_set!(base + a as usize, Value::bool(result)),
                Err(e) => return Err(e),
            }
        }
    }

    // Gt (15)
    15 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::bool(l > r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.compare_gt(left, right) {
                Ok(result) => reg_set!(base + a as usize, Value::bool(result)),
                Err(e) => return Err(e),
            }
        }
    }

    // Ge (16)
    16 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::bool(l >= r));
        } else {
            self.frames[current_frame_idx].ip = ip;
            match self.compare_ge(left, right) {
                Ok(result) => reg_set!(base + a as usize, Value::bool(result)),
                Err(e) => return Err(e),
            }
        }
    }

    // LtII (59)
    59 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l < r));
    }

    // LeII (60)
    60 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l <= r));
    }

    // GtII (61)
    61 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l > r));
    }

    // GeII (62)
    62 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l >= r));
    }

    // EqII (63)
    63 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l == r));
    }

    // NeII (64)
    64 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        let r = reg_ref!(base + c as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l != r));
    }

    // LtFF (65)
    65 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_float_unchecked();
        let r = reg_ref!(base + c as usize).as_float_unchecked();
        reg_set!(base + a as usize, Value::bool(l < r));
    }

    // LeFF (66)
    66 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_float_unchecked();
        let r = reg_ref!(base + c as usize).as_float_unchecked();
        reg_set!(base + a as usize, Value::bool(l <= r));
    }

    // GtFF (67)
    67 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_float_unchecked();
        let r = reg_ref!(base + c as usize).as_float_unchecked();
        reg_set!(base + a as usize, Value::bool(l > r));
    }

    // GeFF (68)
    68 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_float_unchecked();
        let r = reg_ref!(base + c as usize).as_float_unchecked();
        reg_set!(base + a as usize, Value::bool(l >= r));
    }

    // EqFF (69)
    69 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_float_unchecked();
        let r = reg_ref!(base + c as usize).as_float_unchecked();
        reg_set!(base + a as usize, Value::bool(l == r));
    }

    // NeFF (70)
    70 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_float_unchecked();
        let r = reg_ref!(base + c as usize).as_float_unchecked();
        reg_set!(base + a as usize, Value::bool(l != r));
    }

    // LtIImm (71)
    71 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l < c as i64));
    }

    // LeIImm (72)
    72 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l <= c as i64));
    }

    // GtIImm (73)
    73 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l > c as i64));
    }

    // GeIImm (74)
    74 => {
        let (a, b, c) = decode_abc(instr);
        let l = reg_ref!(base + b as usize).as_int_unchecked();
        reg_set!(base + a as usize, Value::bool(l >= c as i64));
    }

    // LtIIG (92)
    92 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::bool(l < r));
        } else {
            let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
            let r_float = right
                .as_float()
                .or_else(|| right.as_int().map(|i| i as f64));
            if let (Some(l), Some(r)) = (l_float, r_float) {
                reg_set!(base + a as usize, Value::bool(l < r));
            } else {
                reg_set!(base + a as usize, Value::bool(false));
            }
        }
    }

    // LeIIG (93)
    93 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::bool(l <= r));
        } else {
            let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
            let r_float = right
                .as_float()
                .or_else(|| right.as_int().map(|i| i as f64));
            if let (Some(l), Some(r)) = (l_float, r_float) {
                reg_set!(base + a as usize, Value::bool(l <= r));
            } else {
                reg_set!(base + a as usize, Value::bool(false));
            }
        }
    }

    // GtIIG (94)
    94 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::bool(l > r));
        } else {
            let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
            let r_float = right
                .as_float()
                .or_else(|| right.as_int().map(|i| i as f64));
            if let (Some(l), Some(r)) = (l_float, r_float) {
                reg_set!(base + a as usize, Value::bool(l > r));
            } else {
                reg_set!(base + a as usize, Value::bool(false));
            }
        }
    }

    // GeIIG (95)
    95 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::bool(l >= r));
        } else {
            let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
            let r_float = right
                .as_float()
                .or_else(|| right.as_int().map(|i| i as f64));
            if let (Some(l), Some(r)) = (l_float, r_float) {
                reg_set!(base + a as usize, Value::bool(l >= r));
            } else {
                reg_set!(base + a as usize, Value::bool(false));
            }
        }
    }

    // EqIIG (96)
    96 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::bool(l == r));
        } else {
            let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
            let r_float = right
                .as_float()
                .or_else(|| right.as_int().map(|i| i as f64));
            if let (Some(l), Some(r)) = (l_float, r_float) {
                reg_set!(base + a as usize, Value::bool(l == r));
            } else {
                let eq = if left == right {
                    true
                } else if let (Some(lp), Some(rp)) = (left.as_ptr(), right.as_ptr()) {
                    let l_ref = GcRef::new(lp);
                    let r_ref = GcRef::new(rp);
                    match (self.heap.get(l_ref), self.heap.get(r_ref)) {
                        (Some(lo), Some(ro)) => match (&lo.kind, &ro.kind) {
                            (ObjectKind::String(ls), ObjectKind::String(rs)) => ls == rs,
                            _ => false,
                        },
                        _ => false,
                    }
                } else {
                    false
                };
                reg_set!(base + a as usize, Value::bool(eq));
            }
        }
    }

    // NeIIG (97)
    97 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        if let (Some(l), Some(r)) = (left.as_int(), right.as_int()) {
            reg_set!(base + a as usize, Value::bool(l != r));
        } else {
            let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
            let r_float = right
                .as_float()
                .or_else(|| right.as_int().map(|i| i as f64));
            if let (Some(l), Some(r)) = (l_float, r_float) {
                reg_set!(base + a as usize, Value::bool(l != r));
            } else {
                let eq = if left == right {
                    true
                } else if let (Some(lp), Some(rp)) = (left.as_ptr(), right.as_ptr()) {
                    let l_ref = GcRef::new(lp);
                    let r_ref = GcRef::new(rp);
                    match (self.heap.get(l_ref), self.heap.get(r_ref)) {
                        (Some(lo), Some(ro)) => match (&lo.kind, &ro.kind) {
                            (ObjectKind::String(ls), ObjectKind::String(rs)) => ls == rs,
                            _ => false,
                        },
                        _ => false,
                    }
                } else {
                    false
                };
                reg_set!(base + a as usize, Value::bool(!eq));
            }
        }
    }

    // LtFFG (98)
    98 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
        let r_float = right
            .as_float()
            .or_else(|| right.as_int().map(|i| i as f64));
        if let (Some(l), Some(r)) = (l_float, r_float) {
            reg_set!(base + a as usize, Value::bool(l < r));
        } else {
            reg_set!(base + a as usize, Value::bool(false));
        }
    }

    // LeFFG (99)
    99 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
        let r_float = right
            .as_float()
            .or_else(|| right.as_int().map(|i| i as f64));
        if let (Some(l), Some(r)) = (l_float, r_float) {
            reg_set!(base + a as usize, Value::bool(l <= r));
        } else {
            reg_set!(base + a as usize, Value::bool(false));
        }
    }

    // GtFFG (100)
    100 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
        let r_float = right
            .as_float()
            .or_else(|| right.as_int().map(|i| i as f64));
        if let (Some(l), Some(r)) = (l_float, r_float) {
            reg_set!(base + a as usize, Value::bool(l > r));
        } else {
            reg_set!(base + a as usize, Value::bool(false));
        }
    }

    // GeFFG (101)
    101 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
        let r_float = right
            .as_float()
            .or_else(|| right.as_int().map(|i| i as f64));
        if let (Some(l), Some(r)) = (l_float, r_float) {
            reg_set!(base + a as usize, Value::bool(l >= r));
        } else {
            reg_set!(base + a as usize, Value::bool(false));
        }
    }

    // EqFFG (102)
    102 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
        let r_float = right
            .as_float()
            .or_else(|| right.as_int().map(|i| i as f64));
        if let (Some(l), Some(r)) = (l_float, r_float) {
            reg_set!(base + a as usize, Value::bool(l == r));
        } else {
            let eq = if left == right {
                true
            } else if let (Some(lp), Some(rp)) = (left.as_ptr(), right.as_ptr()) {
                let l_ref = GcRef::new(lp);
                let r_ref = GcRef::new(rp);
                match (self.heap.get(l_ref), self.heap.get(r_ref)) {
                    (Some(lo), Some(ro)) => match (&lo.kind, &ro.kind) {
                        (ObjectKind::String(ls), ObjectKind::String(rs)) => ls == rs,
                        _ => false,
                    },
                    _ => false,
                }
            } else {
                false
            };
            reg_set!(base + a as usize, Value::bool(eq));
        }
    }

    // NeFFG (103)
    103 => {
        let (a, b, c) = decode_abc(instr);
        let left = reg_get!(base + b as usize);
        let right = reg_get!(base + c as usize);
        let l_float = left.as_float().or_else(|| left.as_int().map(|i| i as f64));
        let r_float = right
            .as_float()
            .or_else(|| right.as_int().map(|i| i as f64));
        if let (Some(l), Some(r)) = (l_float, r_float) {
            reg_set!(base + a as usize, Value::bool(l != r));
        } else {
            let eq = if left == right {
                true
            } else if let (Some(lp), Some(rp)) = (left.as_ptr(), right.as_ptr()) {
                let l_ref = GcRef::new(lp);
                let r_ref = GcRef::new(rp);
                match (self.heap.get(l_ref), self.heap.get(r_ref)) {
                    (Some(lo), Some(ro)) => match (&lo.kind, &ro.kind) {
                        (ObjectKind::String(ls), ObjectKind::String(rs)) => ls == rs,
                        _ => false,
                    },
                    _ => false,
                }
            } else {
                false
            };
            reg_set!(base + a as usize, Value::bool(!eq));
        }
    }

    _ => unreachable!(),
}

// Load/Store operations: Move(0), LoadI(1), LoadK(2), LoadNull(3), LoadBool(4),
// GetGlobalIdx(75), SetGlobalIdx(76)

match opcode_byte {
    // Move (0)
    0 => {
        let (a, b, _) = decode_abc(instr);
        let val = reg_get!(base + b as usize);
        reg_set!(base + a as usize, val);
    }

    // LoadI (1)
    1 => {
        let (a, imm) = decode_aimm(instr);
        reg_set!(base + a as usize, Value::int(imm as i64));
    }

    // LoadK (2)
    2 => {
        let (a, imm) = decode_aimm(instr);
        let k = imm as u16 as usize;

        // SAFETY: Validate index before accessing constants array
        if k >= constants_len {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidConstantIndex {
                index: k,
                max: constants_len,
            }));
        }
        let constant = unsafe { *constants_ptr.add(k) };

        // Check if nested function (rare case)
        if let Some(ptr_val) = constant.as_ptr() {
            if (ptr_val & (1 << 20)) != 0 {
                // Slow path for nested functions
                self.frames[current_frame_idx].ip = ip;
                let func_idx = ptr_val & 0xFFFFF;
                match self.get_nested_function(func_ref, func_idx) {
                    Ok(nested_func) => {
                        self.verify_function_value(&nested_func)?;
                        self.maybe_collect();
                        let func_obj_ref = match self.alloc_function(nested_func) {
                            Ok(func_obj_ref) => func_obj_ref,
                            Err(e) => return Err(e),
                        };
                        reg_set!(base + a as usize, Value::ptr(func_obj_ref.index()));
                        continue;
                    }
                    Err(e) => return Err(e),
                }
            }
        }
        reg_set!(base + a as usize, constant);
    }

    // LoadNull (3)
    3 => {
        let (a, _, _) = decode_abc(instr);
        reg_set!(base + a as usize, Value::null());
    }

    // LoadBool (4)
    4 => {
        let (a, b, _) = decode_abc(instr);
        reg_set!(base + a as usize, Value::bool(b != 0));
    }

    // GetGlobalIdx (75)
    75 => {
        let (a, imm) = decode_aimm(instr);
        let idx = imm as u16 as usize;
        let value = if idx < self.globals_by_index.len() {
            self.globals_by_index[idx]
        } else {
            Value::null()
        };
        reg_set!(base + a as usize, value);
    }

    // SetGlobalIdx (76)
    76 => {
        let (a, imm) = decode_aimm(instr);
        let idx = imm as u16 as usize;
        let value = reg_get!(base + a as usize);
        self.set_global_by_index(idx, value);
    }

    _ => unreachable!(),
}

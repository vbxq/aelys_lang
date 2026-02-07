// Global variable operations: GetGlobal(24), SetGlobal(25), IncGlobalI(39)

match opcode_byte {
    // GetGlobal (24)
    24 => {
        let (a, imm) = decode_aimm(instr);
        let name_index = imm as u16 as usize;
        if name_index >= constants_len {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidConstantIndex {
                index: name_index,
                max: constants_len,
            }));
        }
        let name_constant = unsafe { *constants_ptr.add(name_index) };
        let name_ptr = match name_constant.as_ptr() {
            Some(p) => p,
            None => {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                    "GetGlobal constant is not a string pointer".to_string(),
                )));
            }
        };

        // Get the string from the heap
        let name = match self.heap.get(GcRef::new(name_ptr)) {
            Some(obj) => {
                if let ObjectKind::String(s) = &obj.kind {
                    s.as_str()
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                        "GetGlobal constant is not a string".to_string(),
                    )));
                }
            }
            None => {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                    "GetGlobal constant points to invalid heap object".to_string(),
                )));
            }
        };

        self.frames[current_frame_idx].ip = ip;
        match self.get_global(name) {
            Some(value) => {
                reg_set!(base + a as usize, value);
            }
            None => {
                return Err(self.runtime_error(RuntimeErrorKind::UndefinedVariable(
                    name.to_string(),
                )));
            }
        }
    }

    // SetGlobal (25)
    25 => {
        let (a, imm) = decode_aimm(instr);
        let name_index = imm as u16 as usize;
        if name_index >= constants_len {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidConstantIndex {
                index: name_index,
                max: constants_len,
            }));
        }
        let name_constant = unsafe { *constants_ptr.add(name_index) };
        let name_ptr = match name_constant.as_ptr() {
            Some(p) => p,
            None => {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                    "SetGlobal constant is not a string pointer".to_string(),
                )));
            }
        };

        // Get the string from the heap
        let name = match self.heap.get(GcRef::new(name_ptr)) {
            Some(obj) => {
                if let ObjectKind::String(s) = &obj.kind {
                    s.as_str().to_string()
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                        "SetGlobal constant is not a string".to_string(),
                    )));
                }
            }
            None => {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                    "SetGlobal constant points to invalid heap object".to_string(),
                )));
            }
        };

        let value = reg_get!(base + a as usize);
        self.frames[current_frame_idx].ip = ip;
        self.set_global(name, value);
    }

    // IncGlobalI (39)
    39 => {
        let (a, imm) = decode_aimm(instr);
        let idx = imm as u16 as usize;
        if idx >= self.globals_by_index.len() {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                format!("IncGlobalI index {} out of bounds (max: {})", idx, self.globals_by_index.len()),
            )));
        }
        let val = self.globals_by_index[idx];
        if let Some(n) = val.as_int() {
            self.set_global_by_index(idx, Value::int(n.wrapping_add(a as i64)));
        }
    }

    _ => unreachable!(),
}

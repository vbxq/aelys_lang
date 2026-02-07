// Memory operations: EnterNoGc(26), ExitNoGc(27), Alloc(28), Free(29),
// LoadMem(30), LoadMemI(31), StoreMem(32), StoreMemI(33)

match opcode_byte {
    // EnterNoGc (26)
    26 => {
        self.no_gc_depth += 1;
    }

    // ExitNoGc (27)
    27 => {
        if self.no_gc_depth > 0 {
            self.no_gc_depth -= 1;
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                "no_gc underflow: ExitNoGc without matching EnterNoGc".to_string(),
            )));
        }
    }

    // Alloc (28)
    28 => {
        let (a, b, _) = decode_abc(instr);
        let size_val = reg_get!(base + b as usize);

        let size = match size_val.as_int() {
            Some(s) if s >= 0 => s as usize,
            _ => {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "alloc",
                    expected: "non-negative integer",
                    got: self.value_type_name(size_val).to_string(),
                }));
            }
        };

        self.frames[current_frame_idx].ip = ip;
        self.maybe_collect();
        match self.manual_alloc(size, ip as u32) {
            Ok(handle) => {
                reg_set!(base + a as usize, Value::int(handle as i64));
            }
            Err(e) => return Err(e),
        }
    }

    // Free (29)
    29 => {
        let (a, _, _) = decode_abc(instr);
        let ptr_val = reg_get!(base + a as usize);

        if let Some(handle) = ptr_val.as_int()
            && handle >= 0 {
                self.frames[current_frame_idx].ip = ip;
                self.manual_free(handle as usize, ip as u32)?;
            }
    }

    // LoadMem (30)
    30 => {
        let (a, b, c) = decode_abc(instr);
        let ptr_val = reg_get!(base + b as usize);
        let offset_val = reg_get!(base + c as usize);

        let handle = match ptr_val.as_int() {
            Some(h) if h >= 0 => h as usize,
            _ => {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "load",
                    expected: "non-negative integer handle",
                    got: self.value_type_name(ptr_val).to_string(),
                }));
            }
        };

        let offset = match offset_val.as_int() {
            Some(o) if o >= 0 => o as usize,
            _ => {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "load",
                    expected: "non-negative integer offset",
                    got: self.value_type_name(offset_val).to_string(),
                }));
            }
        };

        self.frames[current_frame_idx].ip = ip;
        match self.manual_heap().load(handle, offset) {
            Ok(value) => {
                reg_set!(base + a as usize, value);
            }
            Err(e) => return Err(self.manual_heap_error(e)),
        }
    }

    // LoadMemI (31)
    31 => {
        let (a, b, c) = decode_abc(instr);
        let ptr_val = reg_get!(base + b as usize);
        let offset = c as usize;

        let handle = match ptr_val.as_int() {
            Some(h) if h >= 0 => h as usize,
            _ => {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "load",
                    expected: "non-negative integer handle",
                    got: self.value_type_name(ptr_val).to_string(),
                }));
            }
        };

        self.frames[current_frame_idx].ip = ip;
        match self.manual_heap().load(handle, offset) {
            Ok(value) => {
                reg_set!(base + a as usize, value);
            }
            Err(e) => return Err(self.manual_heap_error(e)),
        }
    }

    // StoreMem (32)
    32 => {
        let (a, b, c) = decode_abc(instr);
        let ptr_val = reg_get!(base + a as usize);
        let offset_val = reg_get!(base + b as usize);
        let value = reg_get!(base + c as usize);

        let handle = match ptr_val.as_int() {
            Some(h) if h >= 0 => h as usize,
            _ => {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "store",
                    expected: "non-negative integer handle",
                    got: self.value_type_name(ptr_val).to_string(),
                }));
            }
        };

        let offset = match offset_val.as_int() {
            Some(o) if o >= 0 => o as usize,
            _ => {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "store",
                    expected: "non-negative integer offset",
                    got: self.value_type_name(offset_val).to_string(),
                }));
            }
        };

        self.frames[current_frame_idx].ip = ip;
        if let Err(e) = self.manual_heap_mut().store(handle, offset, value) {
            return Err(self.manual_heap_error(e));
        }
    }

    // StoreMemI (33)
    33 => {
        let (a, b, c) = decode_abc(instr);
        let ptr_val = reg_get!(base + a as usize);
        let offset = b as usize;
        let value = reg_get!(base + c as usize);

        let handle = match ptr_val.as_int() {
            Some(h) if h >= 0 => h as usize,
            _ => {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "store",
                    expected: "non-negative integer handle",
                    got: self.value_type_name(ptr_val).to_string(),
                }));
            }
        };

        self.frames[current_frame_idx].ip = ip;
        if let Err(e) = self.manual_heap_mut().store(handle, offset, value) {
            return Err(self.manual_heap_error(e));
        }
    }

    _ => unreachable!(),
}

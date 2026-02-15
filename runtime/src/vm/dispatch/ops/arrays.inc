match opcode_byte {
    130 => {
        let (a, b, _) = decode_abc(instr);
        let dest = base + a as usize;
        let count = reg_get!(base + b as usize).as_int().unwrap_or(0) as usize;
        let array = AelysArray::new_ints(count);
        self.frames[current_frame_idx].ip = ip;
        match self.alloc_array(array) {
            Ok(arr_ref) => {
                reg_set!(dest, Value::ptr(arr_ref.index()));
            }
            Err(e) => return Err(e),
        }
    }

    131 => {
        let (a, b, _) = decode_abc(instr);
        let dest = base + a as usize;
        let count = reg_get!(base + b as usize).as_int().unwrap_or(0) as usize;
        let array = AelysArray::new_floats(count);
        self.frames[current_frame_idx].ip = ip;
        match self.alloc_array(array) {
            Ok(arr_ref) => {
                reg_set!(dest, Value::ptr(arr_ref.index()));
            }
            Err(e) => return Err(e),
        }
    }

    132 => {
        let (a, b, _) = decode_abc(instr);
        let dest = base + a as usize;
        let count = reg_get!(base + b as usize).as_int().unwrap_or(0) as usize;
        let array = AelysArray::new_bools(count);
        self.frames[current_frame_idx].ip = ip;
        match self.alloc_array(array) {
            Ok(arr_ref) => {
                reg_set!(dest, Value::ptr(arr_ref.index()));
            }
            Err(e) => return Err(e),
        }
    }

    133 => {
        let (a, b, _) = decode_abc(instr);
        let dest = base + a as usize;
        let count = reg_get!(base + b as usize).as_int().unwrap_or(0) as usize;
        let array = AelysArray::new_objects(count);
        self.frames[current_frame_idx].ip = ip;
        match self.alloc_array(array) {
            Ok(arr_ref) => {
                reg_set!(dest, Value::ptr(arr_ref.index()));
            }
            Err(e) => return Err(e),
        }
    }

    134 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let start = base + b as usize;
        let count = c as usize;

        self.frames[current_frame_idx].ip = ip;

        if count == 0 {
            let array = AelysArray::new_ints(0);
            match self.alloc_array(array) {
                Ok(arr_ref) => {
                    reg_set!(dest, Value::ptr(arr_ref.index()));
                }
                Err(e) => return Err(e),
            }
        } else {
            let first = reg_get!(start);
            let array = if first.is_int() {
                let mut data = Vec::with_capacity(count);
                for i in 0..count {
                    data.push(reg_get!(start + i).as_int().unwrap_or(0));
                }
                AelysArray::from_ints(data)
            } else if first.is_float() {
                let mut data = Vec::with_capacity(count);
                for i in 0..count {
                    data.push(reg_get!(start + i).as_float().unwrap_or(0.0));
                }
                AelysArray::from_floats(data)
            } else if first.is_bool() {
                let mut data = Vec::with_capacity(count);
                for i in 0..count {
                    data.push(reg_get!(start + i).as_bool().unwrap_or(false));
                }
                AelysArray::from_bools(data)
            } else {
                let mut data = Vec::with_capacity(count);
                for i in 0..count {
                    data.push(reg_get!(start + i));
                }
                AelysArray::from_objects(data)
            };
            match self.alloc_array(array) {
                Ok(arr_ref) => {
                    reg_set!(dest, Value::ptr(arr_ref.index()));
                }
                Err(e) => return Err(e),
            }
        }
    }

    135 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let arr_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(arr_ref) {
            if let ObjectKind::Array(arr) = &obj.kind {
                if let Some(val) = arr.get(idx as usize) {
                    reg_set!(dest, val);
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: arr.len() as i64 }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "array load",
                    expected: "array",
                    got: "non-array object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    136 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let arr_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(arr_ref) {
            if let ObjectKind::Array(arr) = &obj.kind {
                if let Some(val) = arr.get(idx as usize) {
                    reg_set!(dest, val);
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: arr.len() as i64 }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "array load",
                    expected: "array",
                    got: "non-array object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    137 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let arr_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(arr_ref) {
            if let ObjectKind::Array(arr) = &obj.kind {
                if let Some(val) = arr.get(idx as usize) {
                    reg_set!(dest, val);
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: arr.len() as i64 }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "array load",
                    expected: "array",
                    got: "non-array object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    138 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let arr_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(arr_ref) {
            if let ObjectKind::Array(arr) = &obj.kind {
                if let Some(val) = arr.get(idx as usize) {
                    reg_set!(dest, val);
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: arr.len() as i64 }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "array load",
                    expected: "array",
                    got: "non-array object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    139 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let arr_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            reg_set!(dest, Value::null());
        } else {
            let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
            if let Some(obj) = self.heap.get(arr_ref) {
                if let ObjectKind::Array(arr) = &obj.kind {
                    let val = arr.get(idx as usize).unwrap_or(Value::null());
                    reg_set!(dest, val);
                } else {
                    reg_set!(dest, Value::null());
                }
            } else {
                reg_set!(dest, Value::null());
            }
        }
    }

    140 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let arr_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            reg_set!(dest, Value::null());
        } else {
            let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
            if let Some(obj) = self.heap.get(arr_ref) {
                if let ObjectKind::Array(arr) = &obj.kind {
                    let val = arr.get(idx as usize).unwrap_or(Value::null());
                    reg_set!(dest, val);
                } else {
                    reg_set!(dest, Value::null());
                }
            } else {
                reg_set!(dest, Value::null());
            }
        }
    }

    141 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let arr_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            reg_set!(dest, Value::null());
        } else {
            let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
            if let Some(obj) = self.heap.get(arr_ref) {
                if let ObjectKind::Array(arr) = &obj.kind {
                    let val = arr.get(idx as usize).unwrap_or(Value::null());
                    reg_set!(dest, val);
                } else {
                    reg_set!(dest, Value::null());
                }
            } else {
                reg_set!(dest, Value::null());
            }
        }
    }

    142 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let arr_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            reg_set!(dest, Value::null());
        } else {
            let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
            if let Some(obj) = self.heap.get(arr_ref) {
                if let ObjectKind::Array(arr) = &obj.kind {
                    let val = arr.get(idx as usize).unwrap_or(Value::null());
                    reg_set!(dest, val);
                } else {
                    reg_set!(dest, Value::null());
                }
            } else {
                reg_set!(dest, Value::null());
            }
        }
    }

    143 => {
        let (a, b, c) = decode_abc(instr);
        let arr_val = reg_get!(base + a as usize);
        let idx = reg_get!(base + b as usize).as_int().unwrap_or(-1);
        let value = reg_get!(base + c as usize);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
        let arr_len = self.heap.get(arr_ref).and_then(|obj| {
            if let ObjectKind::Array(arr) = &obj.kind { Some(arr.len() as i64) } else { None }
        });

        match arr_len {
            Some(len) => {
                if let Some(obj) = self.heap.get_mut(arr_ref)
                    && let ObjectKind::Array(arr) = &mut obj.kind
                    && !arr.set(idx as usize, value) {
                        self.frames[current_frame_idx].ip = ip;
                        return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: len }));
                    }
            }
            None => {
                if self.heap.get(arr_ref).is_some() {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "array store",
                        expected: "array",
                        got: "non-array object".to_string(),
                    }));
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
                }
            }
        }
    }

    144 => {
        let (a, b, c) = decode_abc(instr);
        let arr_val = reg_get!(base + a as usize);
        let idx = reg_get!(base + b as usize).as_int().unwrap_or(-1);
        let value = reg_get!(base + c as usize);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
        let arr_len = self.heap.get(arr_ref).and_then(|obj| {
            if let ObjectKind::Array(arr) = &obj.kind { Some(arr.len() as i64) } else { None }
        });

        match arr_len {
            Some(len) => {
                if let Some(obj) = self.heap.get_mut(arr_ref)
                    && let ObjectKind::Array(arr) = &mut obj.kind
                    && !arr.set(idx as usize, value) {
                        self.frames[current_frame_idx].ip = ip;
                        return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: len }));
                    }
            }
            None => {
                if self.heap.get(arr_ref).is_some() {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "array store",
                        expected: "array",
                        got: "non-array object".to_string(),
                    }));
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
                }
            }
        }
    }

    145 => {
        let (a, b, c) = decode_abc(instr);
        let arr_val = reg_get!(base + a as usize);
        let idx = reg_get!(base + b as usize).as_int().unwrap_or(-1);
        let value = reg_get!(base + c as usize);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
        let arr_len = self.heap.get(arr_ref).and_then(|obj| {
            if let ObjectKind::Array(arr) = &obj.kind { Some(arr.len() as i64) } else { None }
        });

        match arr_len {
            Some(len) => {
                if let Some(obj) = self.heap.get_mut(arr_ref)
                    && let ObjectKind::Array(arr) = &mut obj.kind
                    && !arr.set(idx as usize, value) {
                        self.frames[current_frame_idx].ip = ip;
                        return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: len }));
                    }
            }
            None => {
                if self.heap.get(arr_ref).is_some() {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "array store",
                        expected: "array",
                        got: "non-array object".to_string(),
                    }));
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
                }
            }
        }
    }

    146 => {
        let (a, b, c) = decode_abc(instr);
        let arr_val = reg_get!(base + a as usize);
        let idx = reg_get!(base + b as usize).as_int().unwrap_or(-1);
        let value = reg_get!(base + c as usize);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
        let arr_len = self.heap.get(arr_ref).and_then(|obj| {
            if let ObjectKind::Array(arr) = &obj.kind { Some(arr.len() as i64) } else { None }
        });

        match arr_len {
            Some(len) => {
                if let Some(obj) = self.heap.get_mut(arr_ref)
                    && let ObjectKind::Array(arr) = &mut obj.kind
                    && !arr.set(idx as usize, value) {
                        self.frames[current_frame_idx].ip = ip;
                        return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: len }));
                    }
            }
            None => {
                if self.heap.get(arr_ref).is_some() {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "array store",
                        expected: "array",
                        got: "non-array object".to_string(),
                    }));
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
                }
            }
        }
    }

    147 => {
        let (a, b, _) = decode_abc(instr);
        let dest = base + a as usize;
        let arr_val = reg_get!(base + b as usize);

        let arr_ref = GcRef::new(arr_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(arr_ref) {
            if let ObjectKind::Array(arr) = &obj.kind {
                reg_set!(dest, Value::int(arr.len() as i64));
            } else if let ObjectKind::Vec(vec) = &obj.kind {
                reg_set!(dest, Value::int(vec.len() as i64));
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "array len",
                    expected: "array or vec",
                    got: "non-array object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    148 => {
        let (a, _, _) = decode_abc(instr);
        let dest = base + a as usize;
        let vec = AelysVec::new_ints();
        self.frames[current_frame_idx].ip = ip;
        match self.alloc_vec(vec) {
            Ok(vec_ref) => {
                reg_set!(dest, Value::ptr(vec_ref.index()));
            }
            Err(e) => return Err(e),
        }
    }

    149 => {
        let (a, _, _) = decode_abc(instr);
        let dest = base + a as usize;
        let vec = AelysVec::new_floats();
        self.frames[current_frame_idx].ip = ip;
        match self.alloc_vec(vec) {
            Ok(vec_ref) => {
                reg_set!(dest, Value::ptr(vec_ref.index()));
            }
            Err(e) => return Err(e),
        }
    }

    150 => {
        let (a, _, _) = decode_abc(instr);
        let dest = base + a as usize;
        let vec = AelysVec::new_bools();
        self.frames[current_frame_idx].ip = ip;
        match self.alloc_vec(vec) {
            Ok(vec_ref) => {
                reg_set!(dest, Value::ptr(vec_ref.index()));
            }
            Err(e) => return Err(e),
        }
    }

    151 => {
        let (a, _, _) = decode_abc(instr);
        let dest = base + a as usize;
        let vec = AelysVec::new_objects();
        self.frames[current_frame_idx].ip = ip;
        match self.alloc_vec(vec) {
            Ok(vec_ref) => {
                reg_set!(dest, Value::ptr(vec_ref.index()));
            }
            Err(e) => return Err(e),
        }
    }

    152 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let start = base + b as usize;
        let count = c as usize;

        self.frames[current_frame_idx].ip = ip;

        if count == 0 {
            let vec = AelysVec::new_ints();
            match self.alloc_vec(vec) {
                Ok(vec_ref) => {
                    reg_set!(dest, Value::ptr(vec_ref.index()));
                }
                Err(e) => return Err(e),
            }
        } else {
            let first = reg_get!(start);
            let vec = if first.is_int() {
                let mut data = Vec::with_capacity(count);
                for i in 0..count {
                    data.push(reg_get!(start + i).as_int().unwrap_or(0));
                }
                AelysVec::from_ints(data)
            } else if first.is_float() {
                let mut data = Vec::with_capacity(count);
                for i in 0..count {
                    data.push(reg_get!(start + i).as_float().unwrap_or(0.0));
                }
                AelysVec::from_floats(data)
            } else if first.is_bool() {
                let mut data = Vec::with_capacity(count);
                for i in 0..count {
                    data.push(reg_get!(start + i).as_bool().unwrap_or(false));
                }
                AelysVec::from_bools(data)
            } else {
                let mut data = Vec::with_capacity(count);
                for i in 0..count {
                    data.push(reg_get!(start + i));
                }
                AelysVec::from_objects(data)
            };
            match self.alloc_vec(vec) {
                Ok(vec_ref) => {
                    reg_set!(dest, Value::ptr(vec_ref.index()));
                }
                Err(e) => return Err(e),
            }
        }
    }

    153 => {
        let (a, b, _) = decode_abc(instr);
        let vec_val = reg_get!(base + a as usize);
        let value = reg_get!(base + b as usize);

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get_mut(vec_ref) {
            if let ObjectKind::Vec(vec) = &mut obj.kind {
                if !vec.push(value) {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "vec push",
                        expected: "matching element type",
                        got: "incompatible type".to_string(),
                    }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec push",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    154 => {
        let (a, b, _) = decode_abc(instr);
        let vec_val = reg_get!(base + a as usize);
        let value = reg_get!(base + b as usize);

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get_mut(vec_ref) {
            if let ObjectKind::Vec(vec) = &mut obj.kind {
                if !vec.push(value) {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "vec push",
                        expected: "matching element type",
                        got: "incompatible type".to_string(),
                    }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec push",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    155 => {
        let (a, b, _) = decode_abc(instr);
        let vec_val = reg_get!(base + a as usize);
        let value = reg_get!(base + b as usize);

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get_mut(vec_ref) {
            if let ObjectKind::Vec(vec) = &mut obj.kind {
                if !vec.push(value) {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "vec push",
                        expected: "matching element type",
                        got: "incompatible type".to_string(),
                    }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec push",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    156 => {
        let (a, b, _) = decode_abc(instr);
        let vec_val = reg_get!(base + a as usize);
        let value = reg_get!(base + b as usize);

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get_mut(vec_ref) {
            if let ObjectKind::Vec(vec) = &mut obj.kind {
                if !vec.push(value) {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "vec push",
                        expected: "matching element type",
                        got: "incompatible type".to_string(),
                    }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec push",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    157 => {
        let (a, b, _) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get_mut(vec_ref) {
            if let ObjectKind::Vec(vec) = &mut obj.kind {
                let val = vec.pop().unwrap_or(Value::null());
                reg_set!(dest, val);
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec pop",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    158 => {
        let (a, b, _) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get_mut(vec_ref) {
            if let ObjectKind::Vec(vec) = &mut obj.kind {
                let val = vec.pop().unwrap_or(Value::null());
                reg_set!(dest, val);
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec pop",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    159 => {
        let (a, b, _) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get_mut(vec_ref) {
            if let ObjectKind::Vec(vec) = &mut obj.kind {
                let val = vec.pop().unwrap_or(Value::null());
                reg_set!(dest, val);
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec pop",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    160 => {
        let (a, b, _) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get_mut(vec_ref) {
            if let ObjectKind::Vec(vec) = &mut obj.kind {
                let val = vec.pop().unwrap_or(Value::null());
                reg_set!(dest, val);
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec pop",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    161 => {
        let (a, b, _) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(vec_ref) {
            if let ObjectKind::Vec(vec) = &obj.kind {
                reg_set!(dest, Value::int(vec.len() as i64));
            } else if let ObjectKind::Array(arr) = &obj.kind {
                reg_set!(dest, Value::int(arr.len() as i64));
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec len",
                    expected: "vec or array",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    162 => {
        let (a, b, _) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(vec_ref) {
            if let ObjectKind::Vec(vec) = &obj.kind {
                reg_set!(dest, Value::int(vec.capacity() as i64));
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec capacity",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    163 => {
        let (a, b, _) = decode_abc(instr);
        let vec_val = reg_get!(base + a as usize);
        let additional = reg_get!(base + b as usize).as_int().unwrap_or(0) as usize;

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get_mut(vec_ref) {
            if let ObjectKind::Vec(vec) = &mut obj.kind {
                vec.reserve(additional);
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec reserve",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    164 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(vec_ref) {
            if let ObjectKind::Vec(vec) = &obj.kind {
                if let Some(val) = vec.get(idx as usize) {
                    reg_set!(dest, val);
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: vec.len() as i64 }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec load",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    165 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(vec_ref) {
            if let ObjectKind::Vec(vec) = &obj.kind {
                if let Some(val) = vec.get(idx as usize) {
                    reg_set!(dest, val);
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: vec.len() as i64 }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec load",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    166 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(vec_ref) {
            if let ObjectKind::Vec(vec) = &obj.kind {
                if let Some(val) = vec.get(idx as usize) {
                    reg_set!(dest, val);
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: vec.len() as i64 }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec load",
                    expected: "vec",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    167 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(vec_ref) {
            if let ObjectKind::Vec(vec) = &obj.kind {
                if let Some(val) = vec.get(idx as usize) {
                    reg_set!(dest, val);
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: vec.len() as i64 }));
                }
            } else if let ObjectKind::Array(arr) = &obj.kind {
                if let Some(val) = arr.get(idx as usize) {
                    reg_set!(dest, val);
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: arr.len() as i64 }));
                }
            } else if let ObjectKind::String(s) = &obj.kind {
                let s_clone = s.as_str().to_string();
                if let Some(ch) = s_clone.chars().nth(idx as usize) {
                    let mut buf = [0u8; 4];
                    let char_str = ch.encode_utf8(&mut buf);
                    self.frames[current_frame_idx].ip = ip;
                    match self.intern_string(char_str) {
                        Ok(str_ref) => {
                            reg_set!(dest, Value::ptr(str_ref.index()));
                        }
                        Err(e) => return Err(e),
                    }
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: s_clone.chars().count() as i64 }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "vec load",
                    expected: "vec, array, or string",
                    got: "non-vec object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    168 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            reg_set!(dest, Value::null());
        } else {
            let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
            if let Some(obj) = self.heap.get(vec_ref) {
                if let ObjectKind::Vec(vec) = &obj.kind {
                    let val = vec.get(idx as usize).unwrap_or(Value::null());
                    reg_set!(dest, val);
                } else {
                    reg_set!(dest, Value::null());
                }
            } else {
                reg_set!(dest, Value::null());
            }
        }
    }

    169 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            reg_set!(dest, Value::null());
        } else {
            let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
            if let Some(obj) = self.heap.get(vec_ref) {
                if let ObjectKind::Vec(vec) = &obj.kind {
                    let val = vec.get(idx as usize).unwrap_or(Value::null());
                    reg_set!(dest, val);
                } else {
                    reg_set!(dest, Value::null());
                }
            } else {
                reg_set!(dest, Value::null());
            }
        }
    }

    170 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            reg_set!(dest, Value::null());
        } else {
            let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
            if let Some(obj) = self.heap.get(vec_ref) {
                if let ObjectKind::Vec(vec) = &obj.kind {
                    let val = vec.get(idx as usize).unwrap_or(Value::null());
                    reg_set!(dest, val);
                } else {
                    reg_set!(dest, Value::null());
                }
            } else {
                reg_set!(dest, Value::null());
            }
        }
    }

    171 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let vec_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            reg_set!(dest, Value::null());
        } else {
            let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
            if let Some(obj) = self.heap.get(vec_ref) {
                if let ObjectKind::Vec(vec) = &obj.kind {
                    let val = vec.get(idx as usize).unwrap_or(Value::null());
                    reg_set!(dest, val);
                } else {
                    reg_set!(dest, Value::null());
                }
            } else {
                reg_set!(dest, Value::null());
            }
        }
    }

    172 => {
        let (a, b, c) = decode_abc(instr);
        let vec_val = reg_get!(base + a as usize);
        let idx = reg_get!(base + b as usize).as_int().unwrap_or(-1);
        let value = reg_get!(base + c as usize);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        let vec_len = self.heap.get(vec_ref).and_then(|obj| {
            if let ObjectKind::Vec(vec) = &obj.kind { Some(vec.len() as i64) } else { None }
        });

        match vec_len {
            Some(len) => {
                if let Some(obj) = self.heap.get_mut(vec_ref)
                    && let ObjectKind::Vec(vec) = &mut obj.kind
                    && !vec.set(idx as usize, value) {
                        self.frames[current_frame_idx].ip = ip;
                        return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: len }));
                    }
            }
            None => {
                if self.heap.get(vec_ref).is_some() {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "vec store",
                        expected: "vec",
                        got: "non-vec object".to_string(),
                    }));
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
                }
            }
        }
    }

    173 => {
        let (a, b, c) = decode_abc(instr);
        let vec_val = reg_get!(base + a as usize);
        let idx = reg_get!(base + b as usize).as_int().unwrap_or(-1);
        let value = reg_get!(base + c as usize);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        let vec_len = self.heap.get(vec_ref).and_then(|obj| {
            if let ObjectKind::Vec(vec) = &obj.kind { Some(vec.len() as i64) } else { None }
        });

        match vec_len {
            Some(len) => {
                if let Some(obj) = self.heap.get_mut(vec_ref)
                    && let ObjectKind::Vec(vec) = &mut obj.kind
                    && !vec.set(idx as usize, value) {
                        self.frames[current_frame_idx].ip = ip;
                        return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: len }));
                    }
            }
            None => {
                if self.heap.get(vec_ref).is_some() {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "vec store",
                        expected: "vec",
                        got: "non-vec object".to_string(),
                    }));
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
                }
            }
        }
    }

    174 => {
        let (a, b, c) = decode_abc(instr);
        let vec_val = reg_get!(base + a as usize);
        let idx = reg_get!(base + b as usize).as_int().unwrap_or(-1);
        let value = reg_get!(base + c as usize);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        let vec_len = self.heap.get(vec_ref).and_then(|obj| {
            if let ObjectKind::Vec(vec) = &obj.kind { Some(vec.len() as i64) } else { None }
        });

        match vec_len {
            Some(len) => {
                if let Some(obj) = self.heap.get_mut(vec_ref)
                    && let ObjectKind::Vec(vec) = &mut obj.kind
                    && !vec.set(idx as usize, value) {
                        self.frames[current_frame_idx].ip = ip;
                        return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: len }));
                    }
            }
            None => {
                if self.heap.get(vec_ref).is_some() {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "vec store",
                        expected: "vec",
                        got: "non-vec object".to_string(),
                    }));
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
                }
            }
        }
    }

    175 => {
        let (a, b, c) = decode_abc(instr);
        let vec_val = reg_get!(base + a as usize);
        let idx = reg_get!(base + b as usize).as_int().unwrap_or(-1);
        let value = reg_get!(base + c as usize);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let vec_ref = GcRef::new(vec_val.as_ptr().unwrap_or(0));
        // Check if it's a Vec or Array and get the length
        let container_len = self.heap.get(vec_ref).and_then(|obj| {
            match &obj.kind {
                ObjectKind::Vec(vec) => Some((true, vec.len() as i64)),
                ObjectKind::Array(arr) => Some((false, arr.len() as i64)),
                _ => None,
            }
        });

        match container_len {
            Some((true, len)) => {
                if let Some(obj) = self.heap.get_mut(vec_ref)
                    && let ObjectKind::Vec(vec) = &mut obj.kind
                    && !vec.set(idx as usize, value) {
                        self.frames[current_frame_idx].ip = ip;
                        return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: len }));
                    }
            }
            Some((false, len)) => {
                if let Some(obj) = self.heap.get_mut(vec_ref)
                    && let ObjectKind::Array(arr) = &mut obj.kind
                    && !arr.set(idx as usize, value) {
                        self.frames[current_frame_idx].ip = ip;
                        return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: len }));
                    }
            }
            None => {
                if self.heap.get(vec_ref).is_some() {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "vec store",
                        expected: "vec or array",
                        got: "non-vec object".to_string(),
                    }));
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
                }
            }
        }
    }

    // StringLoadChar (176)
    176 => {
        let (a, b, c) = decode_abc(instr);
        let dest = base + a as usize;
        let str_val = reg_get!(base + b as usize);
        let idx = reg_get!(base + c as usize).as_int().unwrap_or(-1);

        if idx < 0 {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: 0 }));
        }

        let str_ref = GcRef::new(str_val.as_ptr().unwrap_or(0));
        if let Some(obj) = self.heap.get(str_ref) {
            if let ObjectKind::String(s) = &obj.kind {
                let s_clone = s.as_str().to_string();
                if let Some(ch) = s_clone.chars().nth(idx as usize) {
                    let mut buf = [0u8; 4];
                    let char_str = ch.encode_utf8(&mut buf);
                    self.frames[current_frame_idx].ip = ip;
                    match self.intern_string(char_str) {
                        Ok(new_ref) => {
                            reg_set!(dest, Value::ptr(new_ref.index()));
                        }
                        Err(e) => return Err(e),
                    }
                } else {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::IndexOutOfBounds { index: idx, length: s_clone.chars().count() as i64 }));
                }
            } else {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "string load char",
                    expected: "string",
                    got: "non-string object".to_string(),
                }));
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidMemoryHandle));
        }
    }

    _ => unreachable!(),
}

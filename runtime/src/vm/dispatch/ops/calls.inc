// Call operations: Call(21), Return(22), Return0(23),
// CallGlobal(77), CallGlobalMono(78), CallCached(79),
// CallUpval(80), TailCallUpval(81), CallGlobalNative(104)

match opcode_byte {
    // Call (21)
    21 => {
        let mut dest: u8 = 0;
        let mut func_reg: u8 = 0;
        let mut nargs: u8 = 0;
        let mut callee_ref = GcRef::new(0);
        enum CallData {
            Function {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                bytecode_ptr: *const u32,
                bytecode_len: usize,
                constants_ptr: *const Value,
                constants_len: usize,
            },
            Native {
                native: crate::vm::NativeFunction,
            },
            Closure {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                inner_func: GcRef,
                bytecode_ptr: *const u32,
                bytecode_len: usize,
                constants_ptr: *const Value,
                constants_len: usize,
                upvalues_ptr: *const GcRef,
                upvalues_len: usize,
            },
            Invalid,
        }
        let mut call_data = CallData::Invalid;
        let _ = (dest, func_reg, nargs, callee_ref);
        let _ = &call_data;

        // Part 0: Decode and get function value
        {
            let (dest_tmp, func_reg_tmp, nargs_tmp) = decode_abc(instr);
            dest = dest_tmp;
            func_reg = func_reg_tmp;
            nargs = nargs_tmp;

            // Save IP before call
            self.frames[current_frame_idx].ip = ip;

            // Get function value
            let func_value = reg_get!(base + func_reg as usize);
            let func_ptr = match func_value.as_ptr() {
                Some(p) => p,
                None => {
                    return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                        self.value_type_name(func_value).to_string(),
                    )));
                }
            };

            callee_ref = GcRef::new(func_ptr);
        }

        // Part 1: Determine call type
        {
            call_data = match self.heap.get(callee_ref) {
                Some(obj) => match &obj.kind {
                    ObjectKind::Function(func) => {
                        let bc = &func.function.bytecode;
                        let consts = &func.function.constants;
                        CallData::Function {
                            arity: func.arity(),
                            callee_gmap: self
                                .global_mapping_id_for_layout(&func.function.global_layout),
                            num_regs: func.num_registers(),
                            bytecode_ptr: bc.as_ptr(),
                            bytecode_len: bc.len(),
                            constants_ptr: consts.as_ptr(),
                            constants_len: consts.len(),
                        }
                    }
                    ObjectKind::Native(native) => CallData::Native {
                        native: native.clone(),
                    },
                    ObjectKind::Closure(closure) => {
                        let inner_gmap = self
                            .heap
                            .get(closure.function)
                            .and_then(|inner| {
                                if let ObjectKind::Function(f) = &inner.kind {
                                    Some(self.global_mapping_id_for_layout(
                                        &f.function.global_layout,
                                    ))
                                } else {
                                    None
                                }
                            })
                            .unwrap_or(0);
                        CallData::Closure {
                            arity: closure.arity,
                            callee_gmap: inner_gmap,
                            num_regs: closure.num_registers,
                            inner_func: closure.function,
                            bytecode_ptr: closure.bytecode_ptr,
                            bytecode_len: closure.bytecode_len,
                            constants_ptr: closure.constants_ptr,
                            constants_len: closure.constants_len,
                            upvalues_ptr: closure.upvalues.as_ptr(),
                            upvalues_len: closure.upvalues.len(),
                        }
                    }
                    _ => CallData::Invalid,
                },
                None => {
                    return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                        "invalid reference".to_string(),
                    )));
                }
            };
        }

        // Part 2: Execute call
        {
            match call_data {
                CallData::Function {
                    arity,
                    callee_gmap,
                    num_regs,
                    bytecode_ptr: bc_ptr,
                    bytecode_len: bc_len,
                    constants_ptr: const_ptr,
                    constants_len: const_len,
                } => {
                    self.ensure_function_verified(callee_ref)?;
                    if arity != nargs {
                        return Err(self.runtime_error(RuntimeErrorKind::ArityMismatch {
                            expected: arity,
                            got: nargs,
                        }));
                    }

                    if callee_gmap != 0 && callee_gmap != global_mapping_id {
                        if global_mapping_id != 0 {
                            self.sync_current_function_globals();
                        }
                        self.prepare_globals_for_function(callee_ref);
                    }

                    let new_base = base
                        .checked_add(func_reg as usize)
                        .and_then(|v| v.checked_add(1))
                        .ok_or_else(|| self.runtime_error(RuntimeErrorKind::StackOverflow))?;
                    let needed = new_base
                        .checked_add(num_regs as usize)
                        .ok_or_else(|| self.runtime_error(RuntimeErrorKind::StackOverflow))?;
                    if needed > self.registers.len() {
                        self.registers.resize(needed, Value::null());
                        regs_ptr = self.registers.as_mut_ptr();
                        let _ = regs_ptr;
                    }

                    let mut new_frame = CallFrame::with_return_dest(
                        callee_ref, new_base, dest, bc_ptr, bc_len, const_ptr, const_len,
                        num_regs,
                    );
                    new_frame.global_mapping_id = callee_gmap;

                    if self.frames.len() >= crate::vm::MAX_FRAMES {
                        return Err(self.runtime_error(RuntimeErrorKind::StackOverflow));
                    }
                    self.frames.push(new_frame);

                    current_frame_idx = self.frames.len() - 1;
                    ip = 0;
                    base = new_base;
                    func_ref = callee_ref;
                    bytecode_ptr = bc_ptr;
                    bytecode_len = bc_len;
                    constants_ptr = const_ptr;
                    constants_len = const_len;
                    upvalues_ptr = std::ptr::null();
                    upvalues_len = 0;
                    global_mapping_id = callee_gmap;
                }

                CallData::Native { native } => {
                    if native.arity != nargs {
                        return Err(self.runtime_error(RuntimeErrorKind::ArityMismatch {
                            expected: native.arity,
                            got: nargs,
                        }));
                    }

                    let mut args = Vec::with_capacity(nargs as usize);
                    for i in 0..nargs {
                        args.push(reg_get!(base + func_reg as usize + 1 + i as usize));
                    }

                    match self.call_cached_native(&native, &args) {
                        Ok(result) => {
                            reg_set!(base + dest as usize, result);
                        }
                        Err(e) => return Err(e),
                    }
                }

                CallData::Closure {
                    arity,
                    callee_gmap,
                    num_regs,
                    inner_func,
                    bytecode_ptr: bc_ptr,
                    bytecode_len: bc_len,
                    constants_ptr: const_ptr,
                    constants_len: const_len,
                    upvalues_ptr: upval_ptr,
                    upvalues_len: upval_len,
                } => {
                    self.ensure_function_verified(inner_func)?;
                    if arity != nargs {
                        return Err(self.runtime_error(RuntimeErrorKind::ArityMismatch {
                            expected: arity,
                            got: nargs,
                        }));
                    }

                    if callee_gmap != 0 && callee_gmap != global_mapping_id {
                        if global_mapping_id != 0 {
                            self.sync_current_function_globals();
                        }
                        self.prepare_globals_for_function(inner_func);
                    }

                    let new_base = base
                        .checked_add(func_reg as usize)
                        .and_then(|v| v.checked_add(1))
                        .ok_or_else(|| self.runtime_error(RuntimeErrorKind::StackOverflow))?;
                    let needed = new_base
                        .checked_add(num_regs as usize)
                        .ok_or_else(|| self.runtime_error(RuntimeErrorKind::StackOverflow))?;
                    if needed > self.registers.len() {
                        self.registers.resize(needed, Value::null());
                        regs_ptr = self.registers.as_mut_ptr();
                        let _ = regs_ptr;
                    }

                    let mut new_frame = CallFrame::with_upvalues(
                        inner_func, new_base, dest, bc_ptr, bc_len, const_ptr, const_len,
                        upval_ptr, upval_len, num_regs,
                    );
                    new_frame.global_mapping_id = callee_gmap;

                    if self.frames.len() >= crate::vm::MAX_FRAMES {
                        return Err(self.runtime_error(RuntimeErrorKind::StackOverflow));
                    }
                    self.frames.push(new_frame);

                    current_frame_idx = self.frames.len() - 1;
                    ip = 0;
                    base = new_base;
                    func_ref = inner_func;
                    bytecode_ptr = bc_ptr;
                    bytecode_len = bc_len;
                    constants_ptr = const_ptr;
                    upvalues_ptr = upval_ptr;
                    upvalues_len = upval_len;
                    global_mapping_id = callee_gmap;
                }

                CallData::Invalid => {
                    return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                        "non-callable object".to_string(),
                    )));
                }
            }
        }
    }

    // Return (22)
    22 => {
        let (a, _, _) = decode_abc(instr);
        let result = reg_get!(base + a as usize);

        let dest = self.frames[current_frame_idx].return_dest();
        let current_gmap = self.frames[current_frame_idx].global_mapping_id;

        let caller_gmap = if self.frames.len() > 1 {
            self.frames[current_frame_idx - 1].global_mapping_id
        } else {
            0
        };

        let needs_switch = current_gmap != 0 && current_gmap != caller_gmap;

        if needs_switch && caller_gmap != 0 {
            self.sync_current_function_globals();
        }

        self.frames.pop();

        if self.frames.is_empty() {
            return Ok(result);
        }

        current_frame_idx = self.frames.len() - 1;
        let caller_frame = &self.frames[current_frame_idx];
        let caller_base = caller_frame.base;
        let caller_func = caller_frame.function;
        let caller_ip = caller_frame.ip;
        let caller_bc_ptr = caller_frame.bytecode_ptr;
        let caller_bc_len = caller_frame.bytecode_len;
        let caller_const_ptr = caller_frame.constants_ptr;
        let caller_const_len = caller_frame.constants_len;
        let caller_upval_ptr = caller_frame.upvalues_ptr;
        let caller_upval_len = caller_frame.upvalues_len;
        let caller_gmap_id = caller_frame.global_mapping_id;

        if needs_switch && caller_gmap != 0 {
            self.prepare_globals_for_function(caller_func);
        }

        reg_set!(caller_base + dest as usize, result);

        ip = caller_ip;
        base = caller_base;
        func_ref = caller_func;
        bytecode_ptr = caller_bc_ptr;
        bytecode_len = caller_bc_len;
        constants_ptr = caller_const_ptr;
        constants_len = caller_const_len;
        upvalues_ptr = caller_upval_ptr;
        upvalues_len = caller_upval_len;
        global_mapping_id = caller_gmap_id;
    }

    // Return0 (23)
    23 => {
        let dest = self.frames[current_frame_idx].return_dest();
        let current_gmap = self.frames[current_frame_idx].global_mapping_id;

        let caller_gmap = if self.frames.len() > 1 {
            self.frames[current_frame_idx - 1].global_mapping_id
        } else {
            0
        };

        let needs_switch = current_gmap != 0 && current_gmap != caller_gmap;

        if needs_switch && caller_gmap != 0 {
            self.sync_current_function_globals();
        }

        self.frames.pop();

        if self.frames.is_empty() {
            return Ok(Value::null());
        }

        current_frame_idx = self.frames.len() - 1;
        let caller_frame = &self.frames[current_frame_idx];
        let caller_base = caller_frame.base;
        let caller_func = caller_frame.function;
        let caller_ip = caller_frame.ip;
        let caller_bc_ptr = caller_frame.bytecode_ptr;
        let caller_bc_len = caller_frame.bytecode_len;
        let caller_const_ptr = caller_frame.constants_ptr;
        let caller_const_len = caller_frame.constants_len;
        let caller_upval_ptr = caller_frame.upvalues_ptr;
        let caller_upval_len = caller_frame.upvalues_len;
        let caller_gmap_id = caller_frame.global_mapping_id;

        if needs_switch && caller_gmap != 0 {
            self.prepare_globals_for_function(caller_func);
        }

        reg_set!(caller_base + dest as usize, Value::null());

        ip = caller_ip;
        base = caller_base;
        func_ref = caller_func;
        bytecode_ptr = caller_bc_ptr;
        bytecode_len = caller_bc_len;
        constants_ptr = caller_const_ptr;
        constants_len = caller_const_len;
        upvalues_ptr = caller_upval_ptr;
        upvalues_len = caller_upval_len;
        global_mapping_id = caller_gmap_id;
    }

    // CallGlobal (77) - Slow path: first call, populates cache
    77 => {
        let mut dest: u8 = 0;
        let mut nargs: u8 = 0;
        let mut slot_id: u16 = 0;
        let mut current_func_ptr: usize = 0;
        let mut callee_ref = GcRef::new(0);
        enum CallData {
            Function {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                bc_ptr: *const u32,
                bc_len: usize,
                const_ptr: *const Value,
                const_len: usize,
            },
            Native {
                native: crate::vm::NativeFunction,
            },
            Closure {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                inner_func: GcRef,
                bc_ptr: *const u32,
                bc_len: usize,
                const_ptr: *const Value,
                const_len: usize,
                upval_ptr: *const GcRef,
                upval_len: usize,
            },
            Invalid,
        }
        let mut call_data = CallData::Invalid;
        let _ = (dest, nargs, slot_id, current_func_ptr, callee_ref);
        let _ = &call_data;
        include!("call_global.inc");
    }

    // CallGlobalMono (78) - Fast path with cached pointer
    78 => {
        let mut dest: u8 = 0;
        let mut nargs: u8 = 0;
        let mut slot_id: u16 = 0;
        let mut slot: usize = 0;
        let mut new_func_ptr: usize = 0;
        let mut callee_ref = GcRef::new(0);
        enum CallData {
            Function {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                bc_ptr: *const u32,
                bc_len: usize,
                const_ptr: *const Value,
                const_len: usize,
            },
            Native {
                native: crate::vm::NativeFunction,
            },
            Closure {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                inner_func: GcRef,
                bc_ptr: *const u32,
                bc_len: usize,
                const_ptr: *const Value,
                const_len: usize,
                upval_ptr: *const GcRef,
                upval_len: usize,
            },
            Invalid,
        }
        let mut call_data = CallData::Invalid;
        let _ = (dest, nargs, slot_id, slot, new_func_ptr, callee_ref);
        let _ = &call_data;
        include!("call_global_mono.inc");
    }

    // CallCached (79) - Call with function in register
    79 => {
        let mut dest: u8 = 0;
        let mut func_reg: u8 = 0;
        let mut nargs: u8 = 0;
        let mut callee_ref = GcRef::new(0);
        enum CallCachedData {
            Function {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                bc_ptr: *const u32,
                bc_len: usize,
                const_ptr: *const Value,
                const_len: usize,
            },
            Native {
                native: crate::vm::NativeFunction,
            },
            Closure {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                inner_func: GcRef,
                bc_ptr: *const u32,
                bc_len: usize,
                const_ptr: *const Value,
                const_len: usize,
                upval_ptr: *const GcRef,
                upval_len: usize,
            },
            Invalid,
        }
        let mut call_data = CallCachedData::Invalid;
        let _ = (dest, func_reg, nargs, callee_ref);
        let _ = &call_data;
        include!("call_cached.inc");
    }

    // CallUpval (80) - Call function from upvalue
    80 => {
        let mut dest: u8 = 0;
        let mut upval_idx: u8 = 0;
        let mut nargs: u8 = 0;
        let mut callee_ref = GcRef::new(0);
        enum CallUpvalData {
            Function {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                bc_ptr: *const u32,
                bc_len: usize,
                const_ptr: *const Value,
                const_len: usize,
            },
            Closure {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                inner_func: GcRef,
                bc_ptr: *const u32,
                bc_len: usize,
                const_ptr: *const Value,
                const_len: usize,
                upval_ptr: *const GcRef,
                upval_len: usize,
            },
            Invalid,
        }
        let mut call_data = CallUpvalData::Invalid;
        let _ = (dest, upval_idx, nargs, callee_ref);
        let _ = &call_data;
        include!("call_upval.inc");
    }

    // TailCallUpval (81) - Tail call function from upvalue
    81 => {
        let mut dest: u8 = 0;
        let mut upval_idx: u8 = 0;
        let mut nargs: u8 = 0;
        let mut callee_ref = GcRef::new(0);
        enum TailCallData {
            Function {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                bc_ptr: *const u32,
                bc_len: usize,
                const_ptr: *const Value,
                const_len: usize,
            },
            Closure {
                arity: u8,
                callee_gmap: usize,
                num_regs: u8,
                inner_func: GcRef,
                bc_ptr: *const u32,
                bc_len: usize,
                const_ptr: *const Value,
                const_len: usize,
                upval_ptr: *const GcRef,
                upval_len: usize,
            },
            Invalid,
        }
        let mut call_data = TailCallData::Invalid;
        let _ = (dest, upval_idx, nargs, callee_ref);
        let _ = &call_data;
        include!("tail_call_upval.inc");
    }

    // CallGlobalNative (104) - Fast path for native function calls
    104 => {
        let (dest, global_idx, nargs) = decode_abc(instr);

        // Read cached native pointer from bytecode
        let cache_word_1 = unsafe { *bytecode_ptr.add(ip) };
        let cache_word_2 = unsafe { *bytecode_ptr.add(ip + 1) };

        // Decode: word1 = ptr[31:0], word2 = ptr[47:32](16) | arity(8) | unused(8)
        let native_ptr_low = cache_word_1 as usize;
        let native_ptr_high = ((cache_word_2 >> 16) as usize) << 32;
        let native_ptr = native_ptr_high | native_ptr_low;

        // Check if cache is populated (ptr != 0)
        let native = if native_ptr == 0 {
            // First call: do the slow path lookup and populate cache
            let idx = global_idx as usize;

            // Get the function value at this global index
            let func_value = if idx < self.globals_by_index.len() {
                self.globals_by_index[idx]
            } else {
                self.frames[current_frame_idx].ip = ip + 2;
                return Err(self.runtime_error(RuntimeErrorKind::UndefinedVariable(
                    format!("global index {}", idx),
                )));
            };

            let current_func_ptr = match func_value.as_ptr() {
                Some(ptr) => ptr,
                None => {
                    self.frames[current_frame_idx].ip = ip + 2;
                    return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                        format!("global index {} is not callable", idx),
                    )));
                }
            };

            // Get the native function
            let native_ref = GcRef::new(current_func_ptr);
            let native = match self.heap.get(native_ref) {
                Some(obj) => {
                    if let ObjectKind::Native(n) = &obj.kind {
                        n.clone()
                    } else {
                        self.frames[current_frame_idx].ip = ip + 2;
                        return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                            format!("global index {} is not a native function", idx),
                        )));
                    }
                }
                None => {
                    self.frames[current_frame_idx].ip = ip + 2;
                    return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                        "invalid function reference".to_string(),
                    )));
                }
            };

            // Populate the cache
            let arity = native.arity;
            unsafe {
                let mut_ptr = bytecode_ptr as *mut u32;
                // Write native pointer (48-bit)
                *mut_ptr.add(ip) = (current_func_ptr & 0xFFFFFFFF) as u32;
                *mut_ptr.add(ip + 1) = (((current_func_ptr >> 32) as u32) << 16) | (arity as u32);
            }

            native
        } else {
            // Fast path: cache is populated
            let cached_arity = (cache_word_2 & 0xFF) as u8;

            // Arity check
            if cached_arity != nargs {
                self.frames[current_frame_idx].ip = ip + 2;
                return Err(self.runtime_error(RuntimeErrorKind::ArityMismatch {
                    expected: cached_arity,
                    got: nargs,
                }));
            }

            // Get native function from cached pointer
            let native_ref = GcRef::new(native_ptr);
            match self.heap.get(native_ref) {
                Some(obj) => {
                    if let ObjectKind::Native(n) = &obj.kind {
                        n.clone()
                    } else {
                        self.frames[current_frame_idx].ip = ip + 2;
                        return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                            "cached pointer is not a native function".to_string(),
                        )));
                    }
                }
                None => {
                    self.frames[current_frame_idx].ip = ip + 2;
                    return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                        "invalid native function reference".to_string(),
                    )));
                }
            }
        };

        // Skip cache words
        ip += 2;

        // Call the native function
        let mut args = Vec::with_capacity(nargs as usize);
        for i in 0..nargs {
            args.push(reg_get!(base + dest as usize + 1 + i as usize));
        }
        match self.call_cached_native(&native, &args) {
            Ok(result) => {
                reg_set!(base + dest as usize, result);
            }
            Err(e) => {
                self.frames[current_frame_idx].ip = ip;
                return Err(e);
            }
        }
    }

    _ => unreachable!(),
}

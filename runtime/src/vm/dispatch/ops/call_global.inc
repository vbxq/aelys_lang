// CallGlobal (77) - Slow path: first call, populates cache
// Consolidated from call_global_part_00.rs, call_global_part_01.rs, call_global_part_02.rs
{
    // Part 0: Decode and get function value
    let (dest_tmp, global_idx, nargs_tmp) = decode_abc(instr);
    dest = dest_tmp;
    nargs = nargs_tmp;
    let idx = global_idx as usize;

    // Read slot_id from cache word 2 (func_ptr is 0 on first call)
    let cache_word_2 = unsafe { *bytecode_ptr.add(ip + 1) };
    slot_id = (cache_word_2 & 0xFFFF) as u16;

    // Skip cache words
    ip += 2;
    self.frames[current_frame_idx].ip = ip;

    // Bounds check: patching will access ip-3, ip-2, ip-1
    // If ip < 3, the subtraction would underflow (ip is usize)
    if ip < 3 {
        return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
            format!("invalid bytecode offset for CallGlobal patching: ip={} < 3", ip),
        )));
    }

    // Get the current function value at this global index
    let func_value = if idx < self.globals_by_index.len() {
        self.globals_by_index[idx]
    } else {
        return Err(self.runtime_error(RuntimeErrorKind::UndefinedVariable(
            format!("global index {}", idx),
        )));
    };

    current_func_ptr = match func_value.as_ptr() {
        Some(p) => p,
        None => {
            if func_value.is_null() {
                // Try to get the global name for a better error message
                let global_name =
                    self.heap.get(func_ref).and_then(|obj| match &obj.kind {
                        ObjectKind::Function(f) => {
                            f.function.global_layout.names().get(idx).cloned()
                        }
                        ObjectKind::Closure(c) => {
                            self.heap.get(c.function).and_then(|inner_obj| {
                                if let ObjectKind::Function(f) = &inner_obj.kind {
                                    f.function.global_layout.names().get(idx).cloned()
                                } else {
                                    None
                                }
                            })
                        }
                        _ => None,
                    });
                if let Some(name) = global_name {
                    return Err(self.runtime_error_with_hint(
                        RuntimeErrorKind::UndefinedVariable(name.clone()),
                        &name,
                    ));
                }
            }
            return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                self.value_type_name(func_value).to_string(),
            )));
        }
    };

    callee_ref = GcRef::new(current_func_ptr);

    // Part 1: Determine call type and populate cache
    call_data = match self.heap.get(callee_ref) {
        Some(obj) => match &obj.kind {
            ObjectKind::Function(func) => {
                let bc = &func.function.bytecode;
                let consts = &func.function.constants;
                let arity = func.arity();
                let num_regs = func.num_registers();
                let callee_gmap = self.global_mapping_id_for_layout(&func.function.global_layout);
                let bc_ptr = bc.as_ptr();
                let bc_len = bc.len();
                let const_ptr = consts.as_ptr();
                let const_len = consts.len();

                // Populate Vec cache
                let slot = slot_id as usize;
                if slot >= crate::vm::MAX_CALL_SITE_SLOTS {
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                        format!("call site slot {} exceeds maximum {}", slot, crate::vm::MAX_CALL_SITE_SLOTS)
                    )));
                }
                if slot >= self.call_site_cache.len() {
                    self.call_site_cache.resize(slot + 1, crate::vm::CallSiteCacheEntry::default());
                }
                self.call_site_cache[slot] = crate::vm::CallSiteCacheEntry {
                    bytecode_ptr: bc_ptr,
                    constants_ptr: const_ptr,
                    bytecode_len: bc_len as u32,
                    constants_len: const_len as u16,
                    arity,
                    num_registers: num_regs,
                    callee_gmap,
                    is_closure: false,
                };

                // Patch bytecode: write func_ptr and change opcode to CallGlobalMono
                let (new_word1, new_word2) = encode_cache_words(current_func_ptr, slot_id);
                unsafe {
                    let mut_ptr = bytecode_ptr as *mut u32;
                    *mut_ptr.add(ip - 2) = new_word1;
                    *mut_ptr.add(ip - 1) = new_word2;
                    let old_instr = *mut_ptr.add(ip - 3);
                    *mut_ptr.add(ip - 3) = (old_instr & 0x00FFFFFF) | (78 << 24);
                }

                CallData::Function { arity, callee_gmap, num_regs, bc_ptr, bc_len, const_ptr, const_len }
            }
            ObjectKind::Native(native) => {
                // Patch bytecode to CallGlobalNative (opcode 104)
                // Cache words: [native_ptr_low] [native_ptr_high(16) | arity(8) | unused(8)]
                let native_ptr = current_func_ptr;
                let arity = native.arity;
                unsafe {
                    let mut_ptr = bytecode_ptr as *mut u32;
                    // Write native pointer (48-bit)
                    *mut_ptr.add(ip - 2) = (native_ptr & 0xFFFFFFFF) as u32;
                    *mut_ptr.add(ip - 1) = (((native_ptr >> 32) as u32) << 16) | (arity as u32);
                    // Change opcode to 104 (CallGlobalNative)
                    let old_instr = *mut_ptr.add(ip - 3);
                    *mut_ptr.add(ip - 3) = (old_instr & 0x00FFFFFF) | (104 << 24);
                }
                CallData::Native { native: native.clone() }
            }
            ObjectKind::Closure(closure) => {
                let inner_gmap = self.heap.get(closure.function)
                    .and_then(|inner| {
                        if let ObjectKind::Function(f) = &inner.kind {
                            Some(self.global_mapping_id_for_layout(&f.function.global_layout))
                        } else { None }
                    }).unwrap_or(0);

                let inner_func = closure.function;
                let arity = closure.arity;
                let num_regs = closure.num_registers;
                let bc_ptr = closure.bytecode_ptr;
                let bc_len = closure.bytecode_len;
                let const_ptr = closure.constants_ptr;
                let const_len = closure.constants_len;

                // Populate Vec cache
                let slot = slot_id as usize;
                if slot >= crate::vm::MAX_CALL_SITE_SLOTS {
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                        format!("call site slot {} exceeds maximum {}", slot, crate::vm::MAX_CALL_SITE_SLOTS)
                    )));
                }
                if slot >= self.call_site_cache.len() {
                    self.call_site_cache.resize(slot + 1, crate::vm::CallSiteCacheEntry::default());
                }
                self.call_site_cache[slot] = crate::vm::CallSiteCacheEntry {
                    bytecode_ptr: bc_ptr,
                    constants_ptr: const_ptr,
                    bytecode_len: bc_len as u32,
                    constants_len: const_len as u16,
                    arity,
                    num_registers: num_regs,
                    callee_gmap: inner_gmap,
                    is_closure: true,
                };

                // Patch bytecode
                let (new_word1, new_word2) = encode_cache_words(current_func_ptr, slot_id);
                unsafe {
                    let mut_ptr = bytecode_ptr as *mut u32;
                    *mut_ptr.add(ip - 2) = new_word1;
                    *mut_ptr.add(ip - 1) = new_word2;
                    let old_instr = *mut_ptr.add(ip - 3);
                    *mut_ptr.add(ip - 3) = (old_instr & 0x00FFFFFF) | (78 << 24);
                }

                CallData::Closure {
                    arity, callee_gmap: inner_gmap, num_regs, inner_func,
                    bc_ptr, bc_len, const_ptr, const_len,
                    upval_ptr: closure.upvalues.as_ptr(),
                    upval_len: closure.upvalues.len(),
                }
            }
            _ => CallData::Invalid,
        },
        None => {
            return Err(self.runtime_error(RuntimeErrorKind::NotCallable("invalid reference".to_string())));
        }
    };

    // Part 2: Execute the call
    match call_data {
        CallData::Function { arity, callee_gmap, num_regs, bc_ptr, bc_len, const_ptr, const_len } => {
            self.ensure_function_verified(callee_ref)?;
            if arity != nargs {
                return Err(self.runtime_error(RuntimeErrorKind::ArityMismatch { expected: arity, got: nargs }));
            }
            if callee_gmap != 0 && callee_gmap != global_mapping_id {
                if global_mapping_id != 0 { self.sync_current_function_globals(); }
                self.prepare_globals_for_function(callee_ref);
            }
            let new_base = base
                .checked_add(dest as usize)
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
            let mut new_frame = CallFrame::with_return_dest(callee_ref, new_base, dest, bc_ptr, bc_len, const_ptr, const_len, num_regs);
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
                return Err(self.runtime_error(RuntimeErrorKind::ArityMismatch { expected: native.arity, got: nargs }));
            }
            let mut args = Vec::with_capacity(nargs as usize);
            for i in 0..nargs {
                args.push(reg_get!(base + dest as usize + 1 + i as usize));
            }
            match self.call_cached_native(&native, &args) {
                Ok(result) => { reg_set!(base + dest as usize, result); }
                Err(e) => return Err(e),
            }
        }
        CallData::Closure { arity, callee_gmap, num_regs, inner_func, bc_ptr, bc_len, const_ptr, const_len, upval_ptr, upval_len } => {
            self.ensure_function_verified(inner_func)?;
            if arity != nargs {
                return Err(self.runtime_error(RuntimeErrorKind::ArityMismatch { expected: arity, got: nargs }));
            }
            if callee_gmap != 0 && callee_gmap != global_mapping_id {
                if global_mapping_id != 0 { self.sync_current_function_globals(); }
                self.prepare_globals_for_function(inner_func);
            }
            let new_base = base
                .checked_add(dest as usize)
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
            let mut new_frame = CallFrame::with_upvalues(inner_func, new_base, dest, bc_ptr, bc_len, const_ptr, const_len, upval_ptr, upval_len, num_regs);
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
            constants_len = const_len;
            upvalues_ptr = upval_ptr;
            upvalues_len = upval_len;
            global_mapping_id = callee_gmap;
        }
        CallData::Invalid => {
            return Err(self.runtime_error(RuntimeErrorKind::NotCallable("non-callable".to_string())));
        }
    }
}

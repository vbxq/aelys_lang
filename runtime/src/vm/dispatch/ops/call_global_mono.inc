// CallGlobalMono (78) - Fast path with cached pointer
// Consolidated from call_global_mono_part_00.rs, call_global_mono_part_01.rs, call_global_mono_part_02.rs
{
    // Part 0: Decode and check cache
    let (dest_tmp, global_idx, nargs_tmp) = decode_abc(instr);
    dest = dest_tmp;
    nargs = nargs_tmp;
    let idx = global_idx as usize;

    // Read cached func_ptr and slot_id from bytecode
    let cache_word_1 = unsafe { *bytecode_ptr.add(ip) };
    let cache_word_2 = unsafe { *bytecode_ptr.add(ip + 1) };
    let (cached_func_ptr, slot_id_tmp) = decode_cache_words(cache_word_1, cache_word_2);
    slot_id = slot_id_tmp;

    // Skip cache words and save IP
    ip += 2;
    self.frames[current_frame_idx].ip = ip;

    // Fast path: use cached pointer directly (functions are immutable)
    // Security: Also check call_site_cache bounds - cache may have been invalidated
    // by a global mutation (set_global/set_global_by_index clears call_site_cache)
    let slot_tmp = slot_id as usize;
    if cached_func_ptr != 0 && slot_tmp < self.call_site_cache.len() {
        // CACHE HIT: use Vec[slot] directly

        // SAFETY: bounds checked above
        let cached = unsafe { *self.call_site_cache.get_unchecked(slot_tmp) };

        // validate cache entry: bytecode_ptr is null for default/cleared entries
        // This can happen when cache was cleared by set_global but bytecode is still patched
        // Performance note: this check adds ~1 CPU cycle; branch predictor handles it well
        // since cache entries are almost always valid in steady state.
        if !cached.bytecode_ptr.is_null() {
            // valid cache entry, check arity
            if cached.arity != nargs {
                return Err(self.runtime_error(RuntimeErrorKind::ArityMismatch {
                    expected: cached.arity,
                    got: nargs,
                }));
            }

        let callee_ref_tmp = GcRef::new(cached_func_ptr);

        // Global mapping sync (rarely needed, cold path)
        if cached.callee_gmap != 0 && cached.callee_gmap != global_mapping_id {
            if global_mapping_id != 0 { self.sync_current_function_globals(); }
            self.prepare_globals_for_function(callee_ref_tmp);
        }

        // Set up new frame
        let new_base = base
            .checked_add(dest as usize)
            .and_then(|v| v.checked_add(1))
            .ok_or_else(|| self.runtime_error(RuntimeErrorKind::StackOverflow))?;
        let needed = new_base
            .checked_add(cached.num_registers as usize)
            .ok_or_else(|| self.runtime_error(RuntimeErrorKind::StackOverflow))?;
        if needed > self.registers.len() {
            self.registers.resize(needed, Value::null());
            regs_ptr = self.registers.as_mut_ptr();
            let _ = regs_ptr;
        }

        let bc_len_tmp = cached.bytecode_len as usize;
        let const_len_tmp = cached.constants_len as usize;

        if cached.is_closure {
            // For closures, fetch upvalues (still need heap access for upvalues)
            let (upval_ptr_tmp, upval_len_tmp) = match self.heap.get(callee_ref_tmp) {
                Some(obj) => {
                    if let ObjectKind::Closure(closure) = &obj.kind {
                        (closure.upvalues.as_ptr(), closure.upvalues.len())
                    } else { (std::ptr::null(), 0) }
                }
                None => (std::ptr::null(), 0),
            };

            if !upval_ptr_tmp.is_null() {
                let mut new_frame = CallFrame::with_upvalues(
                    callee_ref_tmp, new_base, dest,
                    cached.bytecode_ptr, bc_len_tmp, cached.constants_ptr, const_len_tmp,
                    upval_ptr_tmp, upval_len_tmp, cached.num_registers,
                );
                new_frame.global_mapping_id = cached.callee_gmap;
                if self.frames.len() >= crate::vm::MAX_FRAMES {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::StackOverflow));
                }
                self.frames.push(new_frame);
                current_frame_idx = self.frames.len() - 1;
                ip = 0;
                base = new_base;
                func_ref = callee_ref_tmp;
                bytecode_ptr = cached.bytecode_ptr;
                bytecode_len = bc_len_tmp;
                constants_ptr = cached.constants_ptr;
                constants_len = const_len_tmp;
                upvalues_ptr = upval_ptr_tmp;
                upvalues_len = upval_len_tmp;
                global_mapping_id = cached.callee_gmap;
                continue;
            }
            // Upvalues fetch failed - fall through to guard failure
        } else {
            // Regular function - ULTRA FAST PATH (no heap access!)
            let mut new_frame = CallFrame::with_return_dest(
                callee_ref_tmp, new_base, dest,
                cached.bytecode_ptr, bc_len_tmp, cached.constants_ptr, const_len_tmp,
                cached.num_registers,
            );
            new_frame.global_mapping_id = cached.callee_gmap;
            if self.frames.len() >= crate::vm::MAX_FRAMES {
                self.frames[current_frame_idx].ip = ip;
                return Err(self.runtime_error(RuntimeErrorKind::StackOverflow));
            }
            self.frames.push(new_frame);
            current_frame_idx = self.frames.len() - 1;
            ip = 0;
            base = new_base;
            func_ref = callee_ref_tmp;
            bytecode_ptr = cached.bytecode_ptr;
            bytecode_len = bc_len_tmp;
            constants_ptr = cached.constants_ptr;
            constants_len = const_len_tmp;
            upvalues_ptr = std::ptr::null();
            upvalues_len = 0;
            global_mapping_id = cached.callee_gmap;
            continue;
        }
        } // end of valid cache entry block (bytecode_ptr not null)
        // Invalid cache entry (bytecode_ptr is null): fall through to cache miss
    }

    // CACHE MISS: cached_func_ptr is 0, cache entry invalid, or closure upvalues fetch failed
    // Get the actual function from globals
    let func_value = if idx < self.globals_by_index.len() {
        self.globals_by_index[idx]
    } else {
        return Err(self.runtime_error(RuntimeErrorKind::UndefinedVariable(
            format!("global index {} out of bounds", idx),
        )));
    };

    new_func_ptr = match func_value.as_ptr() {
        Some(p) => p,
        None => {
            return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                self.value_type_name(func_value).to_string(),
            )));
        }
    };
    callee_ref = GcRef::new(new_func_ptr);
    slot = slot_id as usize;

    // Part 1: Update cache on miss
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

                // Update cache
                if slot >= crate::vm::MAX_CALL_SITE_SLOTS {
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                        format!("call site slot {} exceeds maximum {}", slot, crate::vm::MAX_CALL_SITE_SLOTS)
                    )));
                }
                if slot >= self.call_site_cache.len() {
                    self.call_site_cache.resize(slot + 1, crate::vm::CallSiteCacheEntry::default());
                }
                self.call_site_cache[slot] = crate::vm::CallSiteCacheEntry {
                    bytecode_ptr: bc_ptr, constants_ptr: const_ptr,
                    bytecode_len: bc_len as u32, constants_len: const_len as u16,
                    arity, num_registers: num_regs, callee_gmap, is_closure: false,
                };

                // Repatch bytecode with new func_ptr
                let (new_word1, new_word2) = encode_cache_words(new_func_ptr, slot_id);
                unsafe {
                    let mut_ptr = bytecode_ptr as *mut u32;
                    *mut_ptr.add(ip - 2) = new_word1;
                    *mut_ptr.add(ip - 1) = new_word2;
                }

                CallData::Function { arity, callee_gmap, num_regs, bc_ptr, bc_len, const_ptr, const_len }
            }
            ObjectKind::Native(native) => CallData::Native { native: native.clone() },
            ObjectKind::Closure(closure) => {
                let inner_gmap = self.heap.get(closure.function)
                    .and_then(|inner| if let ObjectKind::Function(f) = &inner.kind {
                        Some(self.global_mapping_id_for_layout(&f.function.global_layout))
                    } else { None }).unwrap_or(0);

                let inner_func = closure.function;
                let arity = closure.arity;
                let num_regs = closure.num_registers;
                let bc_ptr = closure.bytecode_ptr;
                let bc_len = closure.bytecode_len;
                let const_ptr = closure.constants_ptr;
                let const_len = closure.constants_len;

                // Update cache
                if slot >= crate::vm::MAX_CALL_SITE_SLOTS {
                    return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                        format!("call site slot {} exceeds maximum {}", slot, crate::vm::MAX_CALL_SITE_SLOTS)
                    )));
                }
                if slot >= self.call_site_cache.len() {
                    self.call_site_cache.resize(slot + 1, crate::vm::CallSiteCacheEntry::default());
                }
                self.call_site_cache[slot] = crate::vm::CallSiteCacheEntry {
                    bytecode_ptr: bc_ptr, constants_ptr: const_ptr,
                    bytecode_len: bc_len as u32, constants_len: const_len as u16,
                    arity, num_registers: num_regs, callee_gmap: inner_gmap, is_closure: true,
                };

                // Repatch bytecode
                let (new_word1, new_word2) = encode_cache_words(new_func_ptr, slot_id);
                unsafe {
                    let mut_ptr = bytecode_ptr as *mut u32;
                    *mut_ptr.add(ip - 2) = new_word1;
                    *mut_ptr.add(ip - 1) = new_word2;
                }

                CallData::Closure {
                    arity, callee_gmap: inner_gmap, num_regs, inner_func,
                    bc_ptr, bc_len, const_ptr, const_len,
                    upval_ptr: closure.upvalues.as_ptr(), upval_len: closure.upvalues.len(),
                }
            }
            _ => CallData::Invalid,
        },
        None => return Err(self.runtime_error(RuntimeErrorKind::NotCallable("invalid reference".to_string()))),
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
            for i in 0..nargs { args.push(reg_get!(base + dest as usize + 1 + i as usize)); }
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

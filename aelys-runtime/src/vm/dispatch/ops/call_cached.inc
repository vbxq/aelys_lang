// CallCached (79) - Call with function in register
// Consolidated from call_cached_part_00.rs, call_cached_part_01.rs, call_cached_part_02.rs
{
    // Part 0: Decode and get function value
    let (dest_tmp, func_reg_tmp, nargs_tmp) = decode_abc(instr);
    dest = dest_tmp;
    func_reg = func_reg_tmp;
    nargs = nargs_tmp;
    self.frames[current_frame_idx].ip = ip;

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

    // Part 1: Determine call type
    call_data = match self.heap.get(callee_ref) {
        Some(obj) => match &obj.kind {
            ObjectKind::Function(func) => {
                let bc = &func.function.bytecode;
                let consts = &func.function.constants;
                CallCachedData::Function {
                    arity: func.arity(),
                    callee_gmap: self
                        .global_mapping_id_for_layout(&func.function.global_layout),
                    num_regs: func.num_registers(),
                    bc_ptr: bc.as_ptr(),
                    bc_len: bc.len(),
                    const_ptr: consts.as_ptr(),
                    const_len: consts.len(),
                }
            }
            ObjectKind::Native(native) => CallCachedData::Native {
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
                CallCachedData::Closure {
                    arity: closure.arity,
                    callee_gmap: inner_gmap,
                    num_regs: closure.num_registers,
                    inner_func: closure.function,
                    bc_ptr: closure.bytecode_ptr,
                    bc_len: closure.bytecode_len,
                    const_ptr: closure.constants_ptr,
                    const_len: closure.constants_len,
                    upval_ptr: closure.upvalues.as_ptr(),
                    upval_len: closure.upvalues.len(),
                }
            }
            _ => CallCachedData::Invalid,
        },
        None => {
            return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                "invalid reference".to_string(),
            )));
        }
    };

    // Part 2: Execute the call
    match call_data {
        CallCachedData::Function {
            arity,
            callee_gmap,
            num_regs,
            bc_ptr,
            bc_len,
            const_ptr,
            const_len,
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
            upvalues_ptr = std::ptr::null();
            upvalues_len = 0;
            global_mapping_id = callee_gmap;
        }
        CallCachedData::Native { native } => {
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
        CallCachedData::Closure {
            arity,
            callee_gmap,
            num_regs,
            inner_func,
            bc_ptr,
            bc_len,
            const_ptr,
            const_len,
            upval_ptr,
            upval_len,
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
        CallCachedData::Invalid => {
            return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                "non-callable object".to_string(),
            )));
        }
    }
}

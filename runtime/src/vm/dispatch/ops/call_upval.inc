// CallUpval (80) - Call function from upvalue
// Consolidated from call_upval_part_00.rs, call_upval_part_01.rs, call_upval_part_02.rs
{
    // Part 0: Decode and get function from upvalue
    let (dest_tmp, upval_idx_tmp, nargs_tmp) = decode_abc(instr);
    dest = dest_tmp;
    upval_idx = upval_idx_tmp;
    nargs = nargs_tmp;

    self.frames[current_frame_idx].ip = ip;

    if upvalues_len == 0 || (upval_idx as usize) >= upvalues_len {
        return Err(self.runtime_error(RuntimeErrorKind::UndefinedVariable(
            "upvalue".to_string(),
        )));
    }

    let upval_ref = unsafe { *upvalues_ptr.add(upval_idx as usize) };
    let func_value = self.get_upvalue_value(upval_ref);

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
                CallUpvalData::Function {
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
                CallUpvalData::Closure {
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
            _ => CallUpvalData::Invalid,
        },
        None => {
            return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                "invalid reference".to_string(),
            )));
        }
    };

    // Part 2: Execute the call
    match call_data {
        CallUpvalData::Function {
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
        CallUpvalData::Closure {
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
        CallUpvalData::Invalid => {
            return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                "non-callable".to_string(),
            )));
        }
    }
}

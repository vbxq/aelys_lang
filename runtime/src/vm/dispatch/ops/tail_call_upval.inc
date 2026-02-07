// TailCallUpval (81) - Tail call function from upvalue
// Consolidated from tail_call_upval_part_00.rs, tail_call_upval_part_01.rs, tail_call_upval_part_02.rs
{
    // Part 0: Decode and get function from upvalue
    let (dest_tmp, upval_idx_tmp, nargs_tmp) = decode_abc(instr);
    dest = dest_tmp;
    upval_idx = upval_idx_tmp;
    nargs = nargs_tmp;

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
                TailCallData::Function {
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
                TailCallData::Closure {
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
            _ => TailCallData::Invalid,
        },
        None => {
            return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                "invalid reference".to_string(),
            )));
        }
    };

    // Part 2: Execute the tail call
    match call_data {
        #[allow(unused_variables)]
        TailCallData::Function {
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
            // Load callee's globals if mapping changes
            if callee_gmap != 0 && callee_gmap != global_mapping_id {
                if global_mapping_id != 0 {
                    self.sync_current_function_globals();
                }
                self.prepare_globals_for_function(callee_ref);
            }
            // Copy args to start of current frame's register window
            for i in 0..nargs {
                reg_set!(
                    base + i as usize,
                    reg_get!(base + dest as usize + 1 + i as usize)
                );
            }
            // Update current frame in-place
            let frame = &mut self.frames[current_frame_idx];
            frame.function = callee_ref;
            frame.ip = 0;
            frame.bytecode_ptr = bc_ptr;
            frame.bytecode_len = bc_len;
            frame.constants_ptr = const_ptr;
            frame.upvalues_ptr = std::ptr::null();
            frame.upvalues_len = 0;
            frame.num_registers = num_regs;
            frame.global_mapping_id = callee_gmap;
            // Update local state
            ip = 0;
            func_ref = callee_ref;
            bytecode_ptr = bc_ptr;
            bytecode_len = bc_len;
            constants_ptr = const_ptr;
            upvalues_ptr = std::ptr::null();
            upvalues_len = 0;
            global_mapping_id = callee_gmap;
        }
        TailCallData::Closure {
            arity,
            callee_gmap,
            num_regs,
            inner_func,
            bc_ptr,
            bc_len,
            const_ptr,
            const_len: _const_len,
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
            // Load callee's globals if mapping changes
            if callee_gmap != 0 && callee_gmap != global_mapping_id {
                if global_mapping_id != 0 {
                    self.sync_current_function_globals();
                }
                self.prepare_globals_for_function(inner_func);
            }
            for i in 0..nargs {
                reg_set!(
                    base + i as usize,
                    reg_get!(base + dest as usize + 1 + i as usize)
                );
            }
            let frame = &mut self.frames[current_frame_idx];
            frame.function = inner_func;
            frame.ip = 0;
            frame.bytecode_ptr = bc_ptr;
            frame.bytecode_len = bc_len;
            frame.constants_ptr = const_ptr;
            frame.upvalues_ptr = upval_ptr;
            frame.upvalues_len = upval_len;
            frame.num_registers = num_regs;
            frame.global_mapping_id = callee_gmap;
            ip = 0;
            func_ref = inner_func;
            bytecode_ptr = bc_ptr;
            bytecode_len = bc_len;
            constants_ptr = const_ptr;
            upvalues_ptr = upval_ptr;
            upvalues_len = upval_len;
            global_mapping_id = callee_gmap;
        }
        TailCallData::Invalid => {
            return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                "non-callable".to_string(),
            )));
        }
    }
}

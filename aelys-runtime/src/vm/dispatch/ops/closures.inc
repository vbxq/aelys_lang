// Closure operations: Print(34), MakeClosure(35), GetUpval(36),
// SetUpval(37), CloseUpvals(38)

match opcode_byte {
    // Print (34)
    34 => {
        let (a, _, _) = decode_abc(instr);
        let value = reg_get!(base + a as usize);
        self.frames[current_frame_idx].ip = ip;
        self.print_value(value);
    }

    // MakeClosure (35)
    35 => {
        let (dest, const_idx, num_upvalues) = decode_abc(instr);

        // Get the function value from constants (k{const_idx})
        if (const_idx as usize) >= constants_len {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::InvalidConstantIndex {
                index: const_idx as usize,
                max: constants_len,
            }));
        }
        let constant = unsafe { *constants_ptr.add(const_idx as usize) };

        // Check if this is a nested function marker
        let (nested_func_ref, upvalue_descriptors) = if let Some(ptr_val) = constant.as_ptr() {
            if (ptr_val & (1 << 20)) != 0 {
                // Nested function marker: extract index and resolve
                let nested_idx = ptr_val & 0xFFFFF;
                self.frames[current_frame_idx].ip = ip;
                let nested_func = self.get_nested_function(func_ref, nested_idx)?;
                let upvalue_descs = nested_func.upvalue_descriptors.clone();
                self.verify_function_value(&nested_func)?;
                self.maybe_collect();
                let nested_ref = self.alloc_function(nested_func)?;
                (nested_ref, upvalue_descs)
            } else {
                // Direct heap pointer - get function object
                let func_ref = GcRef::new(ptr_val);
                match self.heap.get(func_ref) {
                    Some(obj) => {
                        if let ObjectKind::Function(f) = &obj.kind {
                            (func_ref, f.function.upvalue_descriptors.clone())
                        } else {
                            self.frames[current_frame_idx].ip = ip;
                            return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                                operation: "MakeClosure",
                                expected: "function",
                                got: "non-function object".to_string(),
                            }));
                        }
                    }
                    None => {
                        self.frames[current_frame_idx].ip = ip;
                        return Err(self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                            "MakeClosure function not found in heap".to_string(),
                        )));
                    }
                }
            }
        } else {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "MakeClosure",
                expected: "function pointer",
                got: self.value_type_name(constant).to_string(),
            }));
        };

        // Collect upvalue refs using the descriptors
        let mut upvalue_refs = Vec::with_capacity(num_upvalues as usize);
        for desc in upvalue_descriptors.iter().take(num_upvalues as usize) {
            if desc.is_local {
                // Create or reuse upvalue for local variable
                let upval_ref = self.capture_upvalue(base, desc.index)?;
                upvalue_refs.push(upval_ref);
            } else {
                // Copy upvalue from enclosing function
                if (desc.index as usize) >= upvalues_len {
                    self.frames[current_frame_idx].ip = ip;
                    return Err(self.runtime_error(RuntimeErrorKind::UndefinedVariable(
                        format!("upvalue index {} out of bounds", desc.index),
                    )));
                }
                let parent_upval = unsafe { *upvalues_ptr.add(desc.index as usize) };
                upvalue_refs.push(parent_upval);
            }
        }

        // Get function metadata for the closure
        let (bc_ptr, bc_len, const_ptr, const_len, arity, num_regs) =
            match self.heap.get(nested_func_ref) {
                Some(obj) => {
                    if let ObjectKind::Function(f) = &obj.kind {
                        (
                            f.function.bytecode.as_ptr(),
                            f.function.bytecode.len(),
                            f.function.constants.as_ptr(),
                            f.function.constants.len(),
                            f.function.arity,
                            f.function.num_registers,
                        )
                    } else {
                        (std::ptr::null(), 0, std::ptr::null(), 0, 0, 0)
                    }
                }
                None => (std::ptr::null(), 0, std::ptr::null(), 0, 0, 0),
            };

        // Allocate the closure with proper metadata
        self.frames[current_frame_idx].ip = ip;
        self.maybe_collect();
        let closure = AelysClosure::with_cache(
            nested_func_ref,
            upvalue_refs,
            bc_ptr,
            bc_len,
            const_ptr,
            const_len,
            arity,
            num_regs,
        );
        match self.alloc_object(GcObject::new(ObjectKind::Closure(closure))) {
            Ok(closure_ref) => {
                reg_set!(base + dest as usize, Value::ptr(closure_ref.index()));
            }
            Err(e) => return Err(e),
        }
    }

    // GetUpval (36)
    36 => {
        let (a, b, _) = decode_abc(instr);
        let upval_idx = b as usize;

        if upval_idx >= upvalues_len {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::UndefinedVariable(
                format!("upvalue index {} out of bounds", upval_idx),
            )));
        }

        let upval_ref = unsafe { *upvalues_ptr.add(upval_idx) };
        let value = self.get_upvalue_value(upval_ref);
        reg_set!(base + a as usize, value);
    }

    // SetUpval (37)
    37 => {
        let (a, b, _) = decode_abc(instr);
        let upval_idx = a as usize;
        let value = reg_get!(base + b as usize);

        if upval_idx >= upvalues_len {
            self.frames[current_frame_idx].ip = ip;
            return Err(self.runtime_error(RuntimeErrorKind::UndefinedVariable(
                format!("upvalue index {} out of bounds", upval_idx),
            )));
        }

        let upval_ref = unsafe { *upvalues_ptr.add(upval_idx) };
        self.set_upvalue_value(upval_ref, value);
    }

    // CloseUpvals (38)
    38 => {
        let (a, _, _) = decode_abc(instr);
        let from_slot = base + a as usize;
        self.close_upvalues_from(from_slot);
    }

    _ => unreachable!(),
}

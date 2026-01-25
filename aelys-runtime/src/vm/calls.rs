use super::call_data::CallData;
use super::frame::CallFrame;
use super::{GcRef, ObjectKind, Value};
use super::{StepResult, VM};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

impl VM {
    pub fn call_function(&mut self, dest: u8, func_reg: u8, nargs: u8) -> Result<(), RuntimeError> {
        let base = self.current_frame()?.base;
        let idx = base + func_reg as usize;
        debug_assert!(idx < self.registers.len());
        if idx >= self.registers.len() {
            return Err(self.runtime_error(RuntimeErrorKind::InvalidRegister {
                reg: idx,
                max: self.registers.len().saturating_sub(1),
            }));
        }
        let func_value = self.registers[idx];

        let func_ptr = func_value.as_ptr().ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::NotCallable(
                self.value_type_name(func_value).to_string(),
            ))
        })?;

        let func_ref = GcRef::new(func_ptr);

        // Extract call data without holding the borrow
        let call_data = {
            let obj = self.heap.get(func_ref).ok_or_else(|| {
                self.runtime_error(RuntimeErrorKind::NotCallable(
                    "invalid reference".to_string(),
                ))
            })?;

            match &obj.kind {
                ObjectKind::Function(func) => {
                    let bc = &func.function.bytecode;
                    let consts = &func.function.constants;
                    CallData::Function {
                        func_ref,
                        arity: func.arity(),
                        num_registers: func.num_registers(),
                        bytecode_ptr: bc.as_ptr(),
                        bytecode_len: bc.len(),
                        constants_ptr: consts.as_ptr(),
                        constants_len: consts.len(),
                    }
                }
                ObjectKind::Native(native) => CallData::Native {
                    native: native.clone(),
                },
                ObjectKind::Closure(closure) => CallData::Closure {
                    inner_func_ref: closure.function,
                    arity: closure.arity,
                    num_registers: closure.num_registers,
                    bytecode_ptr: closure.bytecode_ptr,
                    bytecode_len: closure.bytecode_len,
                    constants_ptr: closure.constants_ptr,
                    constants_len: closure.constants_len,
                    upvalues_ptr: closure.upvalues.as_ptr(),
                    upvalues_len: closure.upvalues.len(),
                },
                _ => {
                    return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                        "non-callable object".to_string(),
                    )));
                }
            }
        };

        // Now perform the call with the extracted data
        match call_data {
            CallData::Function {
                func_ref,
                arity,
                num_registers,
                bytecode_ptr,
                bytecode_len,
                constants_ptr,
                constants_len,
            } => {
                self.ensure_function_verified(func_ref)?;
                if arity != nargs {
                    return Err(self.runtime_error(RuntimeErrorKind::ArityMismatch {
                        expected: arity,
                        got: nargs,
                    }));
                }

                let new_base = base + func_reg as usize + 1;
                let needed = new_base + num_registers as usize;
                if needed > self.registers.len() {
                    self.registers.resize(needed, Value::null());
                }

                let new_frame = CallFrame::with_return_dest(
                    func_ref,
                    new_base,
                    dest,
                    bytecode_ptr,
                    bytecode_len,
                    constants_ptr,
                    constants_len,
                    num_registers,
                );
                self.push_frame(new_frame)?;
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
                    args.push(self.read_register(func_reg + 1 + i)?);
                }

                let result = self.call_cached_native(&native, &args)?;
                self.write_register(dest, result)?;
            }

            CallData::Closure {
                inner_func_ref,
                arity,
                num_registers,
                bytecode_ptr,
                bytecode_len,
                constants_ptr,
                constants_len,
                upvalues_ptr,
                upvalues_len,
            } => {
                self.ensure_function_verified(inner_func_ref)?;
                if arity != nargs {
                    return Err(self.runtime_error(RuntimeErrorKind::ArityMismatch {
                        expected: arity,
                        got: nargs,
                    }));
                }

                let new_base = base + func_reg as usize + 1;
                let needed = new_base + num_registers as usize;
                if needed > self.registers.len() {
                    self.registers.resize(needed, Value::null());
                }

                let new_frame = CallFrame::with_upvalues(
                    inner_func_ref,
                    new_base,
                    dest,
                    bytecode_ptr,
                    bytecode_len,
                    constants_ptr,
                    constants_len,
                    upvalues_ptr,
                    upvalues_len,
                    num_registers,
                );
                self.push_frame(new_frame)?;
            }
        }

        Ok(())
    }

    /// Return from a function call with a result value.
    pub fn return_from_call(&mut self, result: Value) -> Result<StepResult, RuntimeError> {
        let frame = self.pop_frame().ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::InvalidBytecode(
                "return with empty call stack".to_string(),
            ))
        })?;
        let dest = frame.return_dest();

        if self.frames.is_empty() {
            return Ok(StepResult::Return(result));
        }

        self.write_register(dest, result)?;
        Ok(StepResult::Continue)
    }
}

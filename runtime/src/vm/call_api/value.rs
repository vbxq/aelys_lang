use super::super::{GcRef, ObjectKind, VM, Value};
use super::kinds::FuncKind;
use aelys_common::error::{RuntimeError, RuntimeErrorKind};
use std::sync::Arc;

impl VM {
    /// Call a function value with the given arguments.
    pub fn call_value(&mut self, func_value: Value, args: &[Value]) -> Result<Value, RuntimeError> {
        let func_ptr = func_value.as_ptr().ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::NotCallable(
                self.value_type_name(func_value).to_string(),
            ))
        })?;

        let func_ref = GcRef::new(func_ptr);

        let func_kind = self.extract_func_kind(func_ref)?;

        match func_kind {
            FuncKind::Native { native } => {
                let nargs = args.len() as u8;
                if native.arity != nargs {
                    return Err(self.runtime_error(RuntimeErrorKind::ArityMismatch {
                        expected: native.arity,
                        got: nargs,
                    }));
                }
                self.call_cached_native(&native, args)
            }
            FuncKind::Function {
                arity,
                num_registers,
                bytecode_ptr,
                bytecode_len,
                constants_ptr,
                constants_len,
                global_layout,
            } => self.call_function_kind(
                func_ref,
                args,
                arity,
                num_registers,
                bytecode_ptr,
                bytecode_len,
                constants_ptr,
                constants_len,
                global_layout,
            ),
            FuncKind::Closure {
                inner_func_ref,
                arity,
                num_registers,
                bytecode_ptr,
                bytecode_len,
                constants_ptr,
                constants_len,
                upvalues,
                global_layout,
            } => self.call_closure_kind(
                inner_func_ref,
                args,
                arity,
                num_registers,
                bytecode_ptr,
                bytecode_len,
                constants_ptr,
                constants_len,
                upvalues,
                global_layout,
            ),
        }
    }

    fn extract_func_kind(&self, func_ref: GcRef) -> Result<FuncKind, RuntimeError> {
        let obj = self.heap.get(func_ref).ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::NotCallable(
                "invalid reference".to_string(),
            ))
        })?;

        match &obj.kind {
            ObjectKind::Function(func) => {
                let bc = &func.function.bytecode;
                let consts = &func.function.constants;
                Ok(FuncKind::Function {
                    arity: func.arity(),
                    num_registers: func.num_registers(),
                    bytecode_ptr: bc.as_ptr(),
                    bytecode_len: bc.len(),
                    constants_ptr: consts.as_ptr(),
                    constants_len: consts.len(),
                    global_layout: Arc::clone(&func.function.global_layout),
                })
            }
            ObjectKind::Native(native) => Ok(FuncKind::Native {
                native: native.clone(),
            }),
            ObjectKind::Closure(closure) => {
                let inner_layout = if let Some(inner_obj) = self.heap.get(closure.function) {
                    if let ObjectKind::Function(f) = &inner_obj.kind {
                        Arc::clone(&f.function.global_layout)
                    } else {
                        return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                            "invalid closure".to_string(),
                        )));
                    }
                } else {
                    return Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                        "invalid closure reference".to_string(),
                    )));
                };
                Ok(FuncKind::Closure {
                    inner_func_ref: closure.function,
                    arity: closure.arity,
                    num_registers: closure.num_registers,
                    bytecode_ptr: closure.bytecode_ptr,
                    bytecode_len: closure.bytecode_len,
                    constants_ptr: closure.constants_ptr,
                    constants_len: closure.constants_len,
                    upvalues: closure.upvalues.clone(),
                    global_layout: inner_layout,
                })
            }
            _ => Err(self.runtime_error(RuntimeErrorKind::NotCallable(
                "non-callable object".to_string(),
            ))),
        }
    }
}

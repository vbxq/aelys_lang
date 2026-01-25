use super::VM;
use super::{Function, GcRef, ObjectKind, UpvalueLocation, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

impl VM {
    pub fn print_value(&self, value: Value) { println!("{}", self.value_to_string(value)); }

    pub fn value_to_string(&self, value: Value) -> String {
        if let Some(ptr) = value.as_ptr() {
            if let Some(obj) = self.heap.get(GcRef::new(ptr)) {
                return object_to_string(self, &obj.kind, value);
            }
        }
        value.to_string()
    }

    /// Get a constant value from a function's constant table.
    pub fn get_constant(&self, func_ref: GcRef, k: u16) -> Result<Value, RuntimeError> {
        let obj = self.heap.get(func_ref).ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "get_constant",
                expected: "valid function",
                got: "invalid reference".to_string(),
            })
        })?;

        if let ObjectKind::Function(func) = &obj.kind {
            func.function
                .constants
                .get(k as usize)
                .copied()
                .ok_or_else(|| {
                    self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "get_constant",
                        expected: "valid constant index",
                        got: k.to_string(),
                    })
                })
        } else {
            Err(self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "get_constant",
                expected: "function",
                got: "non-function".to_string(),
            }))
        }
    }

    /// Get a nested function from a parent function's nested_functions table.
    pub fn get_nested_function(
        &self,
        func_ref: GcRef,
        idx: usize,
    ) -> Result<Function, RuntimeError> {
        let obj = self.heap.get(func_ref).ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "get_nested_function",
                expected: "valid function",
                got: "invalid reference".to_string(),
            })
        })?;

        if let ObjectKind::Function(func) = &obj.kind {
            func.function
                .nested_functions
                .get(idx)
                .cloned()
                .ok_or_else(|| {
                    self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "get_nested_function",
                        expected: "valid function index",
                        got: idx.to_string(),
                    })
                })
        } else {
            Err(self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "get_nested_function",
                expected: "function",
                got: "non-function".to_string(),
            }))
        }
    }

    /// Get a string constant from a function's constant table.
    pub fn get_constant_string(&self, func_ref: GcRef, k: u8) -> Result<String, RuntimeError> {
        let obj = self.heap.get(func_ref).ok_or_else(|| {
            self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "get_constant_string",
                expected: "valid function",
                got: "invalid reference".to_string(),
            })
        })?;

        if let ObjectKind::Function(func) = &obj.kind {
            let constant = func.function.constants.get(k as usize).ok_or_else(|| {
                self.runtime_error(RuntimeErrorKind::TypeError {
                    operation: "get_constant_string",
                    expected: "valid constant index",
                    got: k.to_string(),
                })
            })?;

            if let Some(ptr) = constant.as_ptr() {
                let str_obj = self.heap.get(GcRef::new(ptr)).ok_or_else(|| {
                    self.runtime_error(RuntimeErrorKind::TypeError {
                        operation: "get_constant_string",
                        expected: "valid string",
                        got: "invalid reference".to_string(),
                    })
                })?;

                if let ObjectKind::String(s) = &str_obj.kind {
                    return Ok(s.as_str().to_string());
                }
            }

            Err(self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "get_constant_string",
                expected: "string constant",
                got: self.value_type_name(*constant).to_string(),
            }))
        } else {
            Err(self.runtime_error(RuntimeErrorKind::TypeError {
                operation: "get_constant_string",
                expected: "function",
                got: "non-function".to_string(),
            }))
        }
    }

    /// Get the type name of a value for error messages.
    pub fn value_type_name(&self, value: Value) -> &'static str {
        if value.is_int() {
            return "int";
        }
        if value.is_float() {
            return "float";
        }
        if value.is_bool() {
            return "bool";
        }
        if value.is_null() {
            return "null";
        }
        if let Some(ptr) = value.as_ptr() {
            if let Some(obj) = self.heap.get(GcRef::new(ptr)) {
                return object_type_name(&obj.kind);
            }
            return "invalid reference";
        }
        "unknown"
    }
}

fn object_type_name(kind: &ObjectKind) -> &'static str {
    match kind {
        ObjectKind::String(_) => "string",
        ObjectKind::Function(_) => "function",
        ObjectKind::Native(_) => "native function",
        ObjectKind::Upvalue(_) => "upvalue",
        ObjectKind::Closure(_) => "closure",
    }
}

fn object_to_string(vm: &VM, kind: &ObjectKind, _fallback: Value) -> String {
    match kind {
        ObjectKind::String(s) => s.as_str().to_string(),
        ObjectKind::Function(f) => format!("<function {}>", f.name().unwrap_or("<anonymous>")),
        ObjectKind::Native(n) => format!("<native function {}>", n.name),
        ObjectKind::Upvalue(u) => match &u.location {
            UpvalueLocation::Open {
                frame_base,
                register,
            } => {
                format!("<upvalue open @{}:{}>", frame_base, register)
            }
            UpvalueLocation::Closed(val) => {
                format!("<upvalue closed {}>", vm.value_to_string(*val))
            }
        },
        ObjectKind::Closure(c) => {
            if let Some(func_obj) = vm.heap.get(c.function) {
                if let ObjectKind::Function(f) = &func_obj.kind {
                    return format!("<closure {}>", f.name().unwrap_or("<anonymous>"));
                }
            }
            "<closure>".to_string()
        }
    }
}

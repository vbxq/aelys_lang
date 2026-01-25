use aelys_common::Result;
use aelys_common::error::{AelysError, RuntimeErrorKind};
use aelys_runtime::{self as runtime, VM, Value};

pub fn call_function(vm: &mut VM, name: &str, args: &[Value]) -> Result<Value> {
    vm.call_function_by_name(name, args).map_err(AelysError::Runtime)
}

// get a cached callable for repeated calls (avoids name lookup overhead)
pub fn get_function(vm: &VM, name: &str) -> Result<CallableFunction> {
    let func_value = vm.get_function_value(name).ok_or_else(|| {
        AelysError::Runtime(
            vm.runtime_error(RuntimeErrorKind::UndefinedVariable(format!(
                "function '{}' not found",
                name
            ))),
        )
    })?;

    let func_ptr = func_value.as_ptr().ok_or_else(|| {
        AelysError::Runtime(
            vm.runtime_error(RuntimeErrorKind::NotCallable("not a function".to_string())),
        )
    })?;

    let func_ref = runtime::GcRef::new(func_ptr);

    let obj = vm.heap().get(func_ref).ok_or_else(|| {
        AelysError::Runtime(vm.runtime_error(RuntimeErrorKind::NotCallable(
            "invalid reference".to_string(),
        )))
    })?;

    match &obj.kind {
        runtime::ObjectKind::Function(func) => {
            let bc = &func.function.bytecode;
            let consts = &func.function.constants;
            Ok(CallableFunction {
                kind: CachedFuncKind::Function {
                    func_ref,
                    arity: func.arity(),
                    num_registers: func.num_registers(),
                    bytecode_ptr: bc.as_ptr(),
                    bytecode_len: bc.len(),
                    constants_ptr: consts.as_ptr(),
                    constants_len: consts.len(),
                },
            })
        }
        runtime::ObjectKind::Native(native) => Ok(CallableFunction {
            kind: CachedFuncKind::Native {
                native: native.clone(),
            },
        }),
        runtime::ObjectKind::Closure(closure) => Ok(CallableFunction {
            kind: CachedFuncKind::Closure {
                func_ref: closure.function,
                arity: closure.arity,
                num_registers: closure.num_registers,
                bytecode_ptr: closure.bytecode_ptr,
                bytecode_len: closure.bytecode_len,
                constants_ptr: closure.constants_ptr,
                constants_len: closure.constants_len,
                upvalues_ptr: closure.upvalues.as_ptr(),
                upvalues_len: closure.upvalues.len(),
            },
        }),
        _ => Err(AelysError::Runtime(vm.runtime_error(
            RuntimeErrorKind::NotCallable("not callable".to_string()),
        ))),
    }
}

// pre-extracted metadata for fast calls (no hashmap lookup per call)
#[derive(Clone)]
enum CachedFuncKind {
    Function {
        func_ref: runtime::GcRef,
        arity: u8,
        num_registers: u8,
        bytecode_ptr: *const u32,
        bytecode_len: usize,
        constants_ptr: *const Value,
        constants_len: usize,
    },
    Native {
        native: runtime::NativeFunction,
    },
    Closure {
        func_ref: runtime::GcRef,
        arity: u8,
        num_registers: u8,
        bytecode_ptr: *const u32,
        bytecode_len: usize,
        constants_ptr: *const Value,
        constants_len: usize,
        upvalues_ptr: *const runtime::GcRef,
        upvalues_len: usize,
    },
}

#[derive(Clone)]
pub struct CallableFunction {
    kind: CachedFuncKind,
}

impl CallableFunction {
    pub fn arity(&self) -> u8 {
        match &self.kind {
            CachedFuncKind::Function { arity, .. } | CachedFuncKind::Closure { arity, .. } => *arity,
            CachedFuncKind::Native { native } => native.arity,
        }
    }

    pub fn is_native(&self) -> bool { matches!(self.kind, CachedFuncKind::Native { .. }) }
    pub fn is_closure(&self) -> bool { matches!(self.kind, CachedFuncKind::Closure { .. }) }

    pub fn call(&self, vm: &mut VM, args: &[Value]) -> Result<Value> {
        match &self.kind {
            CachedFuncKind::Function {
                func_ref,
                arity,
                num_registers,
                bytecode_ptr,
                bytecode_len,
                constants_ptr,
                constants_len,
            } => vm.call_cached_function(
                *func_ref,
                *arity,
                *num_registers,
                *bytecode_ptr,
                *bytecode_len,
                *constants_ptr,
                *constants_len,
                args,
            ),
            CachedFuncKind::Native { native } => vm.call_cached_native(native, args),
            CachedFuncKind::Closure {
                func_ref,
                arity,
                num_registers,
                bytecode_ptr,
                bytecode_len,
                constants_ptr,
                constants_len,
                upvalues_ptr,
                upvalues_len,
            } => vm.call_cached_closure(
                *func_ref,
                *arity,
                *num_registers,
                *bytecode_ptr,
                *bytecode_len,
                *constants_ptr,
                *constants_len,
                *upvalues_ptr,
                *upvalues_len,
                args,
            ),
        }
        .map_err(AelysError::Runtime)
    }
}

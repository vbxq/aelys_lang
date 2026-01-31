use super::VM;
use super::Value;
use aelys_common::error::{RuntimeError, RuntimeErrorKind};

// core builtins only - everything else is stdlib
pub fn register_builtins(vm: &mut VM) -> Result<(), RuntimeError> {
    let type_fn = vm.alloc_native("type", 1, builtin_type)?;
    vm.set_global("type".to_string(), Value::ptr(type_fn.index()));

    let alloc_fn = vm.alloc_native("alloc", 1, builtin_alloc)?;
    vm.set_global("alloc".to_string(), Value::ptr(alloc_fn.index()));

    let free_fn = vm.alloc_native("free", 1, builtin_free)?;
    vm.set_global("free".to_string(), Value::ptr(free_fn.index()));

    let load_fn = vm.alloc_native("load", 2, builtin_load)?;
    vm.set_global("load".to_string(), Value::ptr(load_fn.index()));

    let store_fn = vm.alloc_native("store", 3, builtin_store)?;
    vm.set_global("store".to_string(), Value::ptr(store_fn.index()));

    let tostring_fn = vm.alloc_native("__tostring", 1, builtin_tostring)?;
    vm.set_global("__tostring".to_string(), Value::ptr(tostring_fn.index()));

    Ok(())
}

pub fn builtin_type(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let value = args[0];
    let type_name = vm.value_type_name(value);
    let str_ref = vm.alloc_string(type_name)?;
    Ok(Value::ptr(str_ref.index()))
}

pub fn builtin_alloc(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let size = args[0].as_int().ok_or_else(|| {
        vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: "alloc",
            expected: "int",
            got: vm.value_type_name(args[0]).to_string(),
        })
    })?;

    if size <= 0 {
        return Err(vm.runtime_error(RuntimeErrorKind::InvalidAllocationSize { size }));
    }

    let line = vm.current_line();
    let handle = vm.manual_alloc(size as usize, line)?;

    Ok(Value::int(handle as i64))
}

pub fn builtin_free(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    if args[0].is_null() { // free(null) is noop like C
        return Ok(Value::null());
    }

    let handle = args[0].as_int().ok_or_else(|| {
        vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: "free",
            expected: "int (handle)",
            got: vm.value_type_name(args[0]).to_string(),
        })
    })?;

    if handle < 0 {
        return Err(vm.runtime_error(RuntimeErrorKind::NegativeMemoryIndex { value: handle }));
    }

    let line = vm.current_line();
    vm.manual_free(handle as usize, line)?;

    Ok(Value::null())
}

pub fn builtin_load(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = args[0].as_int().ok_or_else(|| {
        vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: "load",
            expected: "int (handle)",
            got: vm.value_type_name(args[0]).to_string(),
        })
    })?;

    let offset = args[1].as_int().ok_or_else(|| {
        vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: "load",
            expected: "int",
            got: vm.value_type_name(args[1]).to_string(),
        })
    })?;

    if handle < 0 {
        return Err(vm.runtime_error(RuntimeErrorKind::NegativeMemoryIndex { value: handle }));
    }
    if offset < 0 {
        return Err(vm.runtime_error(RuntimeErrorKind::NegativeMemoryIndex { value: offset }));
    }

    let value = vm
        .manual_heap()
        .load(handle as usize, offset as usize)
        .map_err(|e| vm.manual_heap_error(e))?;

    Ok(value)
}

pub fn builtin_store(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let handle = args[0].as_int().ok_or_else(|| {
        vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: "store",
            expected: "int (handle)",
            got: vm.value_type_name(args[0]).to_string(),
        })
    })?;

    let offset = args[1].as_int().ok_or_else(|| {
        vm.runtime_error(RuntimeErrorKind::TypeError {
            operation: "store",
            expected: "int",
            got: vm.value_type_name(args[1]).to_string(),
        })
    })?;

    if handle < 0 {
        return Err(vm.runtime_error(RuntimeErrorKind::NegativeMemoryIndex { value: handle }));
    }
    if offset < 0 {
        return Err(vm.runtime_error(RuntimeErrorKind::NegativeMemoryIndex { value: offset }));
    }

    let value = args[2];

    vm.manual_heap_mut()
        .store(handle as usize, offset as usize, value)
        .map_err(|e| vm.manual_heap_error(e))?;

    Ok(Value::null())
}

pub fn builtin_tostring(vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
    let s = vm.value_to_string(args[0]);
    let str_ref = vm.alloc_string(&s)?;
    Ok(Value::ptr(str_ref.index()))
}

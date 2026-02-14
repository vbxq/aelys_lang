use super::{GcRef, ObjectKind, VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};
use aelys_native::{AelysNativeFn, AelysValue, AelysVmApi};

pub type NativeFn = fn(&mut VM, &[Value]) -> Result<Value, RuntimeError>;

#[derive(Clone, Copy)]
pub enum NativeFunctionImpl {
    Rust(NativeFn),
    Foreign(AelysNativeFn),
}

impl NativeFunctionImpl {
    pub fn call(self, vm: &mut VM, args: &[Value]) -> Result<Value, RuntimeError> {
        match self {
            NativeFunctionImpl::Rust(f) => f(vm, args),
            NativeFunctionImpl::Foreign(f) => {
                let mut arg_bits = Vec::with_capacity(args.len());
                for arg in args {
                    arg_bits.push(arg.raw_bits());
                }
                let mut out = Value::null().raw_bits();
                let status = f(
                    vm as *mut VM as *mut std::ffi::c_void,
                    arg_bits.as_ptr(),
                    arg_bits.len(),
                    &mut out as *mut u64,
                );
                if status != 0 {
                    return Err(vm.runtime_error(RuntimeErrorKind::NativeError { code: status }));
                }
                Ok(Value::from_raw(out))
            }
        }
    }
}

/// C callback for native modules to read string values from the VM.
extern "C" fn native_read_string_callback(
    vm: *mut std::ffi::c_void,
    value: AelysValue,
    out_ptr: *mut *const u8,
    out_len: *mut usize,
) -> i32 {
    let vm = unsafe { &*(vm as *const VM) };
    let val = Value::from_raw(value);
    if let Some(ptr_idx) = val.as_ptr()
        && let Some(obj) = vm.heap.get(GcRef::new(ptr_idx))
        && let ObjectKind::String(s) = &obj.kind
    {
        let str_ref = s.as_str();
        unsafe {
            *out_ptr = str_ref.as_ptr();
            *out_len = str_ref.len();
        }
        return 0;
    }
    1
}

// VM API struct to pass to native modules during init
pub fn build_native_vm_api() -> AelysVmApi {
    AelysVmApi {
        api_version: aelys_native::AELYS_API_VERSION,
        size: std::mem::size_of::<AelysVmApi>() as u32,
        register_function: None,
        register_constant: None,
        register_type: None,
        alloc_string: None,
        read_string: Some(native_read_string_callback),
        _reserved: [0; 3],
    }
}

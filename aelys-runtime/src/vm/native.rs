use super::{VM, Value};
use aelys_common::error::{RuntimeError, RuntimeErrorKind};
use aelys_native::AelysNativeFn;

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

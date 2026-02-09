// FFI types for native modules
// bump ABI_VERSION when changing struct layouts

pub use aelys_native_macros::{aelys_export, aelys_module};

pub const AELYS_ABI_VERSION: u32 = 2;
pub const AELYS_API_VERSION: u32 = 1;

pub type AelysValue = u64;

mod abi;
mod hash;
mod macros;
mod value;

pub use abi::{
    AelysExport, AelysExportKind, AelysInitFn, AelysModuleDescriptor, AelysNativeFn,
    AelysRequiredModule, AelysTypeDescriptor, AelysVmApi,
};
pub use hash::{compute_exports_hash, init_descriptor_exports_hash};
pub use value::{
    value_as_bool, value_as_float, value_as_int, value_as_ptr, value_bool, value_float, value_int,
    value_is_bool, value_is_float, value_is_int, value_is_null, value_is_ptr, value_null,
};

use std::sync::OnceLock;

static VM_API: OnceLock<AelysVmApi> = OnceLock::new();

/// Store the VM API provided by the runtime during module init
/// This is called from the generated init function in #[aelys_module]
pub fn store_vm_api(api: &AelysVmApi) {
    let _ = VM_API.set(*api);
}

/// Read a string value from the VM using the stored API.
pub unsafe fn read_string_from_value(
    vm: *mut core::ffi::c_void,
    value: AelysValue,
) -> Option<String> {
    let api = VM_API.get()?;
    let read_fn = api.read_string?;
    let mut ptr: *const u8 = core::ptr::null();
    let mut len: usize = 0;
    let status = read_fn(vm, value, &mut ptr, &mut len);
    if status != 0 || ptr.is_null() {
        return None;
    }
    let bytes = unsafe { core::slice::from_raw_parts(ptr, len) };
    String::from_utf8(bytes.to_vec()).ok()
}

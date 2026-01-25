use aelys_native::{AelysExport, AelysExportKind, AelysModuleDescriptor, AelysValue, AELYS_ABI_VERSION, value_int};
use core::ffi::c_void;

static MODULE_NAME: &[u8] = b"native_test\0";
static MODULE_VERSION: &[u8] = b"0.1.0\0";
static EXPORT_NAME: &[u8] = b"add\0";

extern "C" fn test_add(
    _vm: *mut c_void,
    _args: *const AelysValue,
    _arg_count: usize,
    _out: *mut AelysValue,
) -> i32 {
    unsafe {
        *_out = value_int(10);
    }
    0
}

static EXPORTS: [AelysExport; 1] = [AelysExport {
    name: EXPORT_NAME.as_ptr() as *const i8,
    kind: AelysExportKind::Function,
    arity: 2,
    _padding: [0; 3],
    value: test_add as *const c_void,
}];

#[unsafe(no_mangle)]
pub static mut aelys_module_descriptor: AelysModuleDescriptor = AelysModuleDescriptor {
    abi_version: AELYS_ABI_VERSION,
    descriptor_size: core::mem::size_of::<AelysModuleDescriptor>() as u32,
    module_name: MODULE_NAME.as_ptr() as *const i8,
    module_version: MODULE_VERSION.as_ptr() as *const i8,
    vm_version_min: core::ptr::null(),
    vm_version_max: core::ptr::null(),
    descriptor_hash: 0,
    exports_hash: 0,
    export_count: EXPORTS.len() as u32,
    exports: EXPORTS.as_ptr(),
    required_module_count: 0,
    required_modules: core::ptr::null(),
    init: None,
};

aelys_native::aelys_init_exports_hash!(aelys_module_descriptor);

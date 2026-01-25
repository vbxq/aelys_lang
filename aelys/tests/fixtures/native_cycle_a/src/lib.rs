use aelys_native::{
    value_int, AelysExport, AelysExportKind, AelysModuleDescriptor, AelysRequiredModule,
    AelysValue, AELYS_ABI_VERSION,
};
use core::ffi::c_void;

static MODULE_NAME: &[u8] = b"cycle_a\0";
static MODULE_VERSION: &[u8] = b"0.1.0\0";
static EXPORT_NAME: &[u8] = b"a\0";

static DEP_NAME: &[u8] = b"cycle_b\0";

extern "C" fn cycle_a_fn(
    _vm: *mut c_void,
    _args: *const AelysValue,
    _arg_count: usize,
    out: *mut AelysValue,
) -> i32 {
    unsafe {
        *out = value_int(1);
    }
    0
}

static EXPORTS: [AelysExport; 1] = [AelysExport {
    name: EXPORT_NAME.as_ptr() as *const i8,
    kind: AelysExportKind::Function,
    arity: 0,
    _padding: [0; 3],
    value: cycle_a_fn as *const c_void,
}];

static REQUIRED: [AelysRequiredModule; 1] = [AelysRequiredModule {
    name: DEP_NAME.as_ptr() as *const i8,
    version_req: core::ptr::null(),
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
    required_module_count: REQUIRED.len() as u32,
    required_modules: REQUIRED.as_ptr(),
    init: None,
};

aelys_native::aelys_init_exports_hash!(aelys_module_descriptor);

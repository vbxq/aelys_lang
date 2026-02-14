// C ABI structs for native module descriptors

use crate::{AELYS_ABI_VERSION, AelysValue};
use core::ffi::{c_char, c_void};

pub type AelysNativeFn = extern "C" fn(
    vm: *mut c_void,
    args: *const AelysValue,
    arg_count: usize,
    out: *mut AelysValue,
) -> i32;

pub type AelysInitFn = extern "C" fn(api: *const AelysVmApi) -> i32;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AelysVmApi {
    pub api_version: u32,
    pub size: u32,
    pub register_function:
        Option<extern "C" fn(name: *const c_char, arity: u8, func: AelysNativeFn) -> i32>,
    pub register_constant: Option<extern "C" fn(name: *const c_char, value: AelysValue) -> i32>,
    pub register_type:
        Option<extern "C" fn(name: *const c_char, type_desc: *const AelysTypeDescriptor) -> i32>,
    pub alloc_string:
        Option<extern "C" fn(bytes: *const u8, len: usize, out: *mut AelysValue) -> i32>,
    pub read_string: Option<
        extern "C" fn(
            vm: *mut c_void,
            value: AelysValue,
            out_ptr: *mut *const u8,
            out_len: *mut usize,
        ) -> i32,
    >,
    pub _reserved: [usize; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AelysTypeDescriptor {
    pub size: u32,
    pub drop: Option<extern "C" fn(value: *mut c_void)>,
    pub _reserved: [usize; 2],
}

#[repr(u32)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AelysExportKind {
    Function = 1,
    Constant = 2,
    Type = 3,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AelysExport {
    pub name: *const c_char,
    pub kind: AelysExportKind,
    pub arity: u8,
    pub _padding: [u8; 3],
    pub value: *const c_void,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AelysRequiredModule {
    pub name: *const c_char,
    pub version_req: *const c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct AelysModuleDescriptor {
    pub abi_version: u32,
    pub descriptor_size: u32,
    pub module_name: *const c_char,
    pub module_version: *const c_char,
    pub vm_version_min: *const c_char,
    pub vm_version_max: *const c_char,
    pub descriptor_hash: u64,
    pub exports_hash: u64,
    pub export_count: u32,
    pub exports: *const AelysExport,
    pub required_module_count: u32,
    pub required_modules: *const AelysRequiredModule,
    pub init: Option<AelysInitFn>,
}

impl AelysModuleDescriptor {
    pub const ABI_VERSION: u32 = AELYS_ABI_VERSION;
}

unsafe impl Sync for AelysVmApi {}
unsafe impl Sync for AelysTypeDescriptor {}
unsafe impl Sync for AelysExport {}
unsafe impl Sync for AelysRequiredModule {}
unsafe impl Sync for AelysModuleDescriptor {}

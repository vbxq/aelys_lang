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
    value_as_bool, value_as_float, value_as_int, value_bool, value_float, value_int, value_is_bool,
    value_is_float, value_is_int, value_is_null, value_null,
};

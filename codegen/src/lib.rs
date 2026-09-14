mod api;
mod infra;
mod lowering;

pub mod types;

pub use api::CodegenContext;
pub use infra::error::{AirNodeLocation, AirNodePosition, LlvmBackendError};

pub type CodegenError = LlvmBackendError;

pub(crate) use infra::naming::{is_reserved_bootstrap_builtin, reserved_bootstrap_builtin_message};

pub(crate) fn module_targets_windows(module: &inkwell::module::Module) -> bool {
    module
        .get_triple()
        .as_str()
        .to_str()
        .map_or(false, |t| t.contains("windows"))
}

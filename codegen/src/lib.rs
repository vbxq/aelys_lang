mod api;
mod infra;
mod lowering;

pub mod types;

pub use api::CodegenContext;
pub use infra::error::{AirNodeLocation, AirNodePosition, LlvmBackendError};

pub type CodegenError = LlvmBackendError;

pub(crate) use infra::naming::{is_reserved_bootstrap_builtin, reserved_bootstrap_builtin_message};

thread_local! {
    static OUTLINE_STR_COUNTS: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

// an asan runtime sees a header read only when the c function makes it
pub fn set_outline_str_counts(outline: Option<bool>) {
    OUTLINE_STR_COUNTS.with(|cell| cell.set(outline));
}

pub(crate) fn outline_str_counts() -> bool {
    OUTLINE_STR_COUNTS
        .with(std::cell::Cell::get)
        .unwrap_or_else(|| std::env::var("AELYS_OUTLINE_STR_COUNTS").as_deref() == Ok("1"))
}

pub(crate) fn module_targets_windows(module: &inkwell::module::Module) -> bool {
    module
        .get_triple()
        .as_str()
        .to_str()
        .map_or(false, |t| t.contains("windows"))
}

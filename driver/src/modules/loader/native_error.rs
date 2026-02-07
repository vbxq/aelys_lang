use super::super::types::ModuleLoader;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_runtime::native::NativeError;
use aelys_syntax::Span;

impl ModuleLoader {
    pub(crate) fn native_error(
        &self,
        module_path: &str,
        err: NativeError,
        span: Span,
    ) -> AelysError {
        AelysError::Compile(CompileError::new(
            CompileErrorKind::InvalidNativeModule {
                module: module_path.to_string(),
                reason: err.to_string(),
            },
            span,
            self.source.clone(),
        ))
    }
}

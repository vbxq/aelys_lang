use super::CompileErrorKind;

impl CompileErrorKind {
    pub fn annotation(&self) -> &'static str {
        match self {
            Self::UndefinedVariable(_) => "not found in this scope",
            Self::VariableAlreadyDefined(_) => "already defined here",
            Self::AssignToImmutable(_) => "variable is not mutable",
            Self::AssignToLoopVariable(_) => "'i' is controlled by the for loop",
            Self::InvalidAssignmentTarget => "cannot assign to this",
            Self::RecursionDepthExceeded { .. } => "nesting limit exceeded",
            Self::CommentNestingTooDeep { .. } => "nesting limit exceeded",
            Self::UnexpectedToken { .. } => "unexpected token",
            Self::ReturnOutsideFunction => "not inside a function",
            Self::IntegerOverflow { .. } => "value exceeds range",
            Self::ModuleNotFound { .. } => "module not found",
            Self::CircularDependency { .. } => "creates circular dependency",
            Self::SymbolNotPublic { .. } => "symbol is not public",
            Self::StdlibNotAvailable { .. } => "'std' modules are not yet implemented",
            Self::SymbolNotFound { .. } => "symbol not found",
            Self::InvalidNativeModule { .. } => "invalid native module",
            Self::NativeCapabilityDenied { .. } => "capability denied",
            Self::NativeChecksumMismatch { .. } => "checksum mismatch",
            Self::NativeVersionMismatch { .. } => "version constraint not satisfied",
            Self::TypeInferenceError(_) => "type inference failed",
            _ => "",
        }
    }
}

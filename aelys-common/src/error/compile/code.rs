use super::CompileErrorKind;

impl CompileErrorKind {
    pub fn code(&self) -> u16 {
        match self {
            Self::UnterminatedString => 1,
            Self::InvalidCharacter(_) => 2,
            Self::InvalidNumber(_) => 3,
            Self::InvalidEscape(_) => 5,
            Self::UnexpectedToken { .. } => 101,
            Self::ExpectedExpression => 102,
            Self::ExpectedIdentifier => 103,
            Self::InvalidAssignmentTarget => 104,
            Self::RecursionDepthExceeded { .. } => 105,
            Self::CommentNestingTooDeep { .. } => 4,
            Self::UndefinedVariable(_) => 201,
            Self::VariableAlreadyDefined(_) => 202,
            Self::AssignToImmutable(_) => 203,
            Self::TooManyConstants => 204,
            Self::TooManyRegisters => 205,
            Self::TooManyArguments => 206,
            Self::TooManyUpvalues => 211,
            Self::BreakOutsideLoop => 207,
            Self::ContinueOutsideLoop => 208,
            Self::AssignToLoopVariable(_) => 210,
            Self::IntegerOverflow { .. } => 209,
            Self::ModuleNotFound { .. } => 401,
            Self::CircularDependency { .. } => 402,
            Self::SymbolNotPublic { .. } => 403,
            Self::StdlibNotAvailable { .. } => 404,
            Self::SymbolNotFound { .. } => 405,
            Self::InvalidNativeModule { .. } => 406,
            Self::NativeCapabilityDenied { .. } => 407,
            Self::NativeChecksumMismatch { .. } => 408,
            Self::NativeVersionMismatch { .. } => 409,
            Self::TypeInferenceError(_) => 301,
        }
    }
}

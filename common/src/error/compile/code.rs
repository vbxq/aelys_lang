use super::CompileErrorKind;

impl CompileErrorKind {
    pub fn code(&self) -> u16 {
        match self {
            // Lexer errors (E00xx)
            Self::UnterminatedString => 1,
            Self::InvalidCharacter(_) => 2,
            Self::InvalidNumber(_) => 3,
            Self::CommentNestingTooDeep { .. } => 4,
            Self::InvalidEscape(_) => 5,
            Self::UnterminatedFmtExpr => 6,
            Self::UnmatchedCloseBrace => 7,

            // Parser errors (E01xx)
            Self::UnexpectedToken { .. } => 101,
            Self::ExpectedExpression => 102,
            Self::ExpectedIdentifier => 103,
            Self::InvalidAssignmentTarget => 104,
            Self::RecursionDepthExceeded { .. } => 105,

            // Name resolution errors (E02xx)
            Self::UndefinedVariable(_) => 201,
            Self::VariableAlreadyDefined(_) => 202,

            // Type errors (E03xx)
            Self::TypeMismatch { .. } => 301,
            Self::ArityMismatch { .. } => 302,
            Self::NotCallable { .. } => 303,
            Self::MemberAccess { .. } => 304,
            Self::InfiniteType { .. } => 305,
            Self::UnknownType { .. } => 306,
            Self::InvalidCast { .. } => 307,
            Self::RecursionLimitExceeded => 309,
            Self::UndefinedFunction(_) => 203,

            // Mutability errors (E04xx)
            Self::AssignToImmutable(_) => 401,
            Self::AssignToLoopVariable(_) => 402,

            // Control flow errors (E05xx)
            Self::BreakOutsideLoop => 501,
            Self::ContinueOutsideLoop => 502,
            Self::ReturnOutsideFunction => 503,

            // Module errors (E04xx range reserved, using 6xx)
            Self::ModuleNotFound { .. } => 601,
            Self::CircularDependency { .. } => 602,
            Self::SymbolNotPublic { .. } => 603,
            Self::StdlibNotAvailable { .. } => 604,
            Self::SymbolNotFound { .. } => 605,
            Self::SymbolConflict { .. } => 610,

            // Sema catch-all (legacy, being phased out)
            Self::TypeInferenceError(_) => 301,

            // Backend errors (E09xx)
            Self::BackendDiagnostic { .. } => 901,
        }
    }
}

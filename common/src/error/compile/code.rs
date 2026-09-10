use super::CompileErrorKind;

impl CompileErrorKind {
    pub fn code(&self) -> u16 {
        match self {
            Self::UnterminatedString => 1,
            Self::InvalidCharacter(_) => 2,
            Self::InvalidNumber(_) => 3,
            Self::CommentNestingTooDeep { .. } => 4,
            Self::InvalidEscape(_) => 5,
            Self::UnterminatedFmtExpr => 6,
            Self::UnmatchedCloseBrace => 7,
            Self::SourceUnreadable { .. } => 8,

            Self::UnexpectedToken { .. } => 101,
            Self::ExpectedExpression => 102,
            Self::ExpectedIdentifier => 103,
            Self::InvalidAssignmentTarget => 104,
            Self::RecursionDepthExceeded { .. } => 105,

            Self::UndefinedVariable(_) => 201,
            Self::VariableAlreadyDefined(_) => 202,
            Self::DuplicateDefinition { .. } => 204,

            Self::TypeMismatch { .. } => 301,
            Self::ArityMismatch { .. } => 302,
            Self::NotCallable { .. } => 303,
            Self::MemberAccess { .. } => 304,
            Self::InfiniteType { .. } => 305,
            Self::UnknownType { .. } => 306,
            Self::InvalidCast { .. } => 307,
            Self::RecursionLimitExceeded => 309,
            Self::UndefinedFunction(_) => 203,

            // mutability errors (e04xx)
            Self::AssignToImmutable(_) => 401,
            Self::AssignToLoopVariable(_) => 402,

            Self::BreakOutsideLoop => 501,
            Self::ContinueOutsideLoop => 502,
            Self::ReturnOutsideFunction => 503,

            Self::ModuleNotFound { .. } => 601,
            Self::AmbiguousModule { .. } => 622,
            Self::CircularDependency { .. } => 602,
            Self::SymbolConflict { .. } => 603,
            Self::ReservedModuleSegment { .. } => 604,
            Self::SymbolNotPublic { .. } => 605,
            Self::SymbolNotFound { .. } => 606,
            Self::ForeignHeaderImport { .. } => 607,
            Self::NeedsOutsidePrologue => 608,
            Self::WildcardImport { .. } => 609,
            Self::ConflictingExternalSymbol { .. } => 612,
            Self::ConflictingForeignDeclarations { .. } => 612,
            Self::ReservedRuntimeSymbol { .. } => 613,
            Self::MalformedForeignDecl { .. } => 614,
            Self::ForeignSignatureType { .. } => 615,
            Self::LinkedLibraryClaimsRuntimeSymbol { .. } => 618,

            Self::TypeInferenceError(_) => 301,

            Self::BackendDiagnostic { fault, .. } => fault.code(),
        }
    }
}

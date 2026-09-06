#[derive(Debug)]
pub enum CompileErrorKind {
    UnterminatedString,
    InvalidCharacter(char),
    InvalidNumber(String),
    InvalidEscape(char),
    UnterminatedFmtExpr,
    UnmatchedCloseBrace,

    UnexpectedToken {
        expected: String,
        found: String,
    },
    ExpectedExpression,
    ExpectedIdentifier,
    InvalidAssignmentTarget,
    RecursionDepthExceeded {
        max: usize,
    },
    CommentNestingTooDeep {
        max: usize,
    },

    UndefinedVariable(String),
    VariableAlreadyDefined(String),
    UndefinedFunction(String),

    TypeMismatch {
        expected: String,
        found: String,
        reason: String,
    },
    ArityMismatch {
        expected: usize,
        found: usize,
        func_name: String,
    },
    NotCallable {
        ty: String,
    },
    MemberAccess {
        message: String,
    },
    InfiniteType {
        message: String,
    },
    UnknownType {
        name: String,
    },
    InvalidCast {
        from: String,
        to: String,
    },
    RecursionLimitExceeded,

    // mutability errors
    AssignToImmutable(String),
    AssignToLoopVariable(String),

    BreakOutsideLoop,
    ContinueOutsideLoop,
    ReturnOutsideFunction,

    ModuleNotFound {
        module_path: String,
        searched_paths: Vec<String>,
    },
    CircularDependency {
        chain: Vec<String>,
    },
    SymbolNotPublic {
        symbol: String,
        module: String,
    },
    ReservedModuleSegment {
        module_path: String,
        segment: String,
    },
    ForeignHeaderImport {
        header: String,
    },
    NeedsOutsidePrologue,
    WildcardImport {
        module_path: String,
    },
    SymbolNotFound {
        symbol: String,
        module: String,
    },
    SymbolConflict {
        symbol: String,
        modules: Vec<String>,
    },
    ConflictingExternalSymbol {
        symbol: String,
    },
    ConflictingForeignDeclarations {
        symbol: String,
        reason: String,
    },
    ReservedRuntimeSymbol {
        symbol: String,
    },
    MalformedForeignDecl {
        reason: String,
    },
    ForeignSignatureType {
        function: String,
        what: String,
        spelling: String,
        reason: String,
    },

    TypeInferenceError(String),

    LinkedLibraryClaimsRuntimeSymbol {
        symbol: String,
        libraries: Vec<String>,
    },

    BackendDiagnostic {
        backend: String,
        message: String,
        note: Option<String>,
        help: Option<String>,
    },
}

#[derive(Debug)]
pub enum CompileErrorKind {
    // Lexer errors
    UnterminatedString,
    InvalidCharacter(char),
    InvalidNumber(String),
    InvalidEscape(char),
    UnterminatedFmtExpr,
    UnmatchedCloseBrace,

    // Parser errors
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

    // Name resolution errors
    UndefinedVariable(String),
    VariableAlreadyDefined(String),
    UndefinedFunction(String),

    // Type errors (individual variants, replacing TypeInferenceError catch-all)
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

    // Mutability errors
    AssignToImmutable(String),
    AssignToLoopVariable(String),

    // Control flow errors
    BreakOutsideLoop,
    ContinueOutsideLoop,
    ReturnOutsideFunction,

    // Module errors
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
    StdlibNotAvailable {
        module: String,
    },
    SymbolNotFound {
        symbol: String,
        module: String,
    },
    SymbolConflict {
        symbol: String,
        modules: Vec<String>,
    },

    /// Legacy catch-all for sema type errors (being phased out in favor of individual variants)
    TypeInferenceError(String),

    BackendDiagnostic {
        backend: String,
        message: String,
        note: Option<String>,
        help: Option<String>,
    },
}

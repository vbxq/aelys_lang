#[derive(Debug)]
pub enum CompileErrorKind {
    // Lexer errors
    UnterminatedString,
    InvalidCharacter(char),
    InvalidNumber(String),
    InvalidEscape(char),

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

    // Compiler errors
    UndefinedVariable(String),
    VariableAlreadyDefined(String),
    AssignToImmutable(String),
    TooManyConstants,
    TooManyRegisters,
    TooManyArguments,
    TooManyUpvalues,
    BreakOutsideLoop,
    ContinueOutsideLoop,
    AssignToLoopVariable(String),

    // 48-bit signed range for NaN-boxed ints
    IntegerOverflow {
        value: String,
        min: i64,
        max: i64,
    },

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
    InvalidNativeModule {
        module: String,
        reason: String,
    },
    NativeCapabilityDenied {
        module: String,
        capability: String,
        required: Vec<String>,
    },
    NativeChecksumMismatch {
        module: String,
        expected: String,
        actual: String,
    },
    NativeVersionMismatch {
        module: String,
        required: String,
        found: Option<String>,
    },

    TypeInferenceError(String),
}

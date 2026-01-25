use super::CompileErrorKind;

impl CompileErrorKind {
    pub fn message(&self) -> String {
        match self {
            Self::UnterminatedString => "unterminated string literal".to_string(),
            Self::InvalidCharacter(c) => format!("invalid character '{}'", c),
            Self::InvalidNumber(s) => format!("invalid number '{}'", s),
            Self::InvalidEscape(c) => format!("invalid escape sequence '\\{}'", c),
            Self::UnexpectedToken { expected, found } => {
                format!("expected {}, found {}", expected, found)
            }
            Self::ExpectedExpression => "expected expression".to_string(),
            Self::ExpectedIdentifier => "expected identifier".to_string(),
            Self::InvalidAssignmentTarget => "invalid assignment target".to_string(),
            Self::RecursionDepthExceeded { max } => {
                format!("expression nesting too deep (max {} levels)", max)
            }
            Self::CommentNestingTooDeep { max } => {
                format!("block comment nesting too deep (max {} levels)", max)
            }
            Self::UndefinedVariable(name) => format!("undefined variable '{}'", name),
            Self::VariableAlreadyDefined(name) => {
                format!("variable '{}' already defined in this scope", name)
            }
            Self::AssignToImmutable(name) => {
                format!("cannot assign to immutable variable '{}'", name)
            }
            Self::TooManyConstants => "too many constants in function".to_string(),
            Self::TooManyRegisters => "too many local variables in function".to_string(),
            Self::TooManyArguments => "too many arguments in function call".to_string(),
            Self::TooManyUpvalues => "too many captured variables (max 255)".to_string(),
            Self::BreakOutsideLoop => "'break' outside of loop".to_string(),
            Self::ContinueOutsideLoop => "'continue' outside of loop".to_string(),
            Self::AssignToLoopVariable(name) => {
                format!("cannot assign to loop variable '{}'", name)
            }
            Self::IntegerOverflow { value, min, max } => format!(
                "integer literal '{}' exceeds 48-bit signed range ({} to {})",
                value, min, max
            ),
            Self::ModuleNotFound {
                module_path,
                searched_paths,
            } => {
                format!(
                    "module not found: '{}'\n   = note: searched in: {}",
                    module_path,
                    searched_paths.join(", ")
                )
            }
            Self::CircularDependency { chain } => {
                format!("circular dependency detected: {}", chain.join(" -> "))
            }
            Self::SymbolNotPublic { symbol, module } => format!(
                "'{}' is not public in module '{}'\n   = help: add 'pub' before the declaration in {}.aelys",
                symbol, module, module
            ),
            Self::StdlibNotAvailable { module } => format!(
                "standard library module '{}' is not yet implemented\n   = note: standard library will be available in a future version",
                module
            ),
            Self::SymbolNotFound { symbol, module } => {
                format!("symbol '{}' not found in module '{}'", symbol, module)
            }
            Self::InvalidNativeModule { module, reason } => {
                format!("invalid native module '{}': {}", module, reason)
            }
            Self::NativeCapabilityDenied {
                module,
                capability,
                required,
            } => {
                let caps_str = required.join(", ");
                format!(
                    "native module '{}' requires capability '{}' which is not allowed\n   \
                     = required capabilities: [{}]\n   \
                     = hint: use --allow-caps={} or -ae.trusted=true to allow",
                    module, capability, caps_str, capability
                )
            }
            Self::NativeChecksumMismatch {
                module,
                expected,
                actual,
            } => format!(
                "native module '{}' checksum mismatch\n   \
                 = expected: {}\n   \
                 = actual:   {}\n   \
                 = hint: the module file may have been modified or corrupted",
                module, expected, actual
            ),
            Self::NativeVersionMismatch {
                module,
                required,
                found,
            } => {
                let found_str = found.as_deref().unwrap_or("(none)");
                format!(
                    "native module '{}' version constraint not satisfied\n   \
                     = required: {}\n   \
                     = found:    {}",
                    module, required, found_str
                )
            }
            Self::TypeInferenceError(msg) => format!("type error: {}", msg),
        }
    }
}

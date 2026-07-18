use super::CompileErrorKind;

impl CompileErrorKind {
    pub fn message(&self) -> String {
        match self {
            Self::UnterminatedString => "unterminated string literal".to_string(),
            Self::InvalidCharacter(c) => format!("invalid character '{}'", c),
            Self::InvalidNumber(s) => format!("invalid number '{}'", s),
            Self::InvalidEscape(c) => format!("invalid escape sequence '\\{}'", c),
            Self::UnterminatedFmtExpr => {
                "unterminated expression in format string (missing '}')".to_string()
            }
            Self::UnmatchedCloseBrace => "unmatched '}' in string (use '}}' to escape)".to_string(),
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
            Self::UndefinedVariable(name) => format!("undefined variable `{}`", name),
            Self::VariableAlreadyDefined(name) => {
                format!("variable `{}` already defined in this scope", name)
            }
            Self::UndefinedFunction(name) => format!("undefined function `{}`", name),
            Self::TypeMismatch {
                expected,
                found,
                reason,
            } => {
                if reason.is_empty() {
                    format!("expected `{}`, found `{}`", expected, found)
                } else {
                    format!("expected `{}`, found `{}` ({})", expected, found, reason)
                }
            }
            Self::ArityMismatch {
                expected,
                found,
                func_name,
            } => format!(
                "function `{}` takes {} argument{} but {} {} supplied",
                func_name,
                expected,
                if *expected == 1 { "" } else { "s" },
                found,
                if *found == 1 { "was" } else { "were" },
            ),
            Self::NotCallable { ty } => format!("type `{}` is not callable", ty),
            Self::MemberAccess { message } => message.clone(),
            Self::InfiniteType { message } => format!("infinite type: {}", message),
            Self::UnknownType { name } => format!("unknown type `{}`", name),
            Self::InvalidCast { from, to } => {
                format!("cannot cast `{}` to `{}`", from, to)
            }
            Self::RecursionLimitExceeded => "type inference recursion limit exceeded".to_string(),
            Self::AssignToImmutable(name) => {
                format!("cannot assign to immutable variable `{}`", name)
            }
            Self::AssignToLoopVariable(name) => {
                format!("cannot assign to loop variable `{}`", name)
            }
            Self::BreakOutsideLoop => "'break' outside of loop".to_string(),
            Self::ContinueOutsideLoop => "'continue' outside of loop".to_string(),
            Self::ReturnOutsideFunction => "'return' outside of function".to_string(),
            Self::ModuleNotFound { module_path, .. } => {
                format!("module not found: '{}'", module_path,)
            }
            Self::CircularDependency { chain } => {
                format!("circular dependency detected: {}", chain.join(" -> "))
            }
            Self::SymbolNotPublic { symbol, module } => {
                format!("'{}' is not public in module '{}'", symbol, module)
            }
            Self::StdlibNotAvailable { module } => {
                format!(
                    "standard library module '{}' is not yet implemented",
                    module
                )
            }
            Self::SymbolNotFound { symbol, module } => {
                format!("symbol '{}' not found in module '{}'", symbol, module)
            }
            Self::SymbolConflict { symbol, modules } => {
                format!(
                    "symbol '{}' is exported by multiple modules: {}",
                    symbol,
                    modules.join(", ")
                )
            }
            Self::TypeInferenceError(msg) => {
                let headline = msg
                    .lines()
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("type inference failed")
                    .trim();
                format!("type error: {}", headline)
            }
            Self::BackendDiagnostic {
                backend, message, ..
            } => format!("[{}] {}", backend, message),
        }
    }
}

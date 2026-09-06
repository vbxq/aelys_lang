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
            Self::ReservedModuleSegment {
                module_path,
                segment,
            } => format!(
                "module path '{}' uses the reserved segment '{}'",
                module_path, segment
            ),
            Self::ForeignHeaderImport { header } => {
                format!(
                    "importing the C header \"{}\" is not implemented yet",
                    header
                )
            }
            Self::NeedsOutsidePrologue => {
                "a `needs` declaration may only appear before any other top-level declaration"
                    .to_string()
            }
            Self::WildcardImport { module_path } => {
                format!(
                    "wildcard import of '{}' is not implemented yet",
                    module_path
                )
            }
            Self::SymbolNotFound { symbol, module } => {
                format!("symbol '{}' not found in module '{}'", symbol, module)
            }
            Self::ConflictingExternalSymbol { symbol } => format!(
                "the external symbol '{}' is also defined in this program, so a call meant for \
                 the foreign function would reach the Aelys body",
                symbol
            ),
            Self::ConflictingForeignDeclarations { symbol, reason } => format!(
                "the external symbol '{}' is declared more than once and the declarations do not \
                 agree: {}",
                symbol, reason
            ),
            Self::ReservedRuntimeSymbol { symbol } => format!(
                "'{}' is a symbol the Aelys runtime links, so the linker would resolve the \
                 runtime's own calls to this function",
                symbol
            ),
            Self::MalformedForeignDecl { reason } => {
                format!("malformed external declaration: {}", reason)
            }
            Self::ForeignSignatureType {
                function,
                what,
                spelling,
                reason,
            } => format!(
                "the external declaration '{}' names the type '{}' for {}, which is outside the \
                 external type surface: {}",
                function, spelling, what, reason
            ),
            Self::SymbolConflict { symbol, modules } => {
                format!(
                    "the import name '{}' is introduced more than once, by: {}",
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
            Self::LinkedLibraryClaimsRuntimeSymbol { symbol, libraries } => format!(
                "the linked executable defines '{}', a symbol the Aelys runtime links, and the \
                 aelys-core archive does not define it; it comes from one of the requested \
                 libraries: {}",
                symbol,
                libraries.join(", ")
            ),
            Self::BackendDiagnostic {
                backend, message, ..
            } => format!("[{}] {}", backend, message),
        }
    }
}

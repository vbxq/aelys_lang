use super::ConstraintReason;
use crate::types::{InferType, TypeVarId};
use aelys_syntax::Span;
use std::fmt;

/// Structured fix suggestion attached to a TypeError
#[derive(Debug, Clone)]
pub struct TypeErrorSuggestion {
    pub message: String,
    pub span: Span,
    pub new_text: String,
}

/// Type error during inference
#[derive(Debug, Clone)]
pub struct TypeError {
    pub kind: TypeErrorKind,
    pub span: Span,
    pub reason: ConstraintReason,
    /// Additional source locations with explanatory labels
    pub secondary_spans: Vec<(Span, String)>,
    /// Inline help text (rendered as `= help:`)
    pub help: Option<String>,
    /// Structured fix suggestion (rendered as `help:` with code diff)
    pub suggestion: Option<TypeErrorSuggestion>,
}

#[derive(Debug, Clone)]
pub enum TypeErrorKind {
    /// Two types could not be unified
    Mismatch {
        expected: InferType,
        found: InferType,
    },
    /// Infinite type (occurs check failed)
    InfiniteType { var: TypeVarId, ty: InferType },
    /// Type is not one of the expected options
    NotOneOf {
        ty: InferType,
        options: Vec<InferType>,
    },
    /// Arity mismatch in function call
    ArityMismatch { expected: usize, found: usize },
    /// Tried to call a non-function
    NotCallable { ty: InferType },
    /// Undefined variable
    UndefinedVariable { name: String },
    /// Undefined function
    UndefinedFunction { name: String },
    /// Invalid field/member access
    MemberAccess { message: String },
    /// Recursion depth limit exceeded in type inference
    RecursionLimit,
    /// Assignment to an immutable variable
    AssignToImmutable {
        name: String,
        binding_span: Option<Span>,
    },
    /// Assignment to a loop-controlled variable
    AssignToLoopVariable { name: String },
}

impl fmt::Display for TypeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.kind {
            TypeErrorKind::Mismatch { expected, found } => {
                write!(
                    f,
                    "type mismatch: expected {}, found {} ({})",
                    expected, found, self.reason
                )
            }
            TypeErrorKind::InfiniteType { var, ty } => {
                write!(f, "infinite type: {} = {} ({})", var, ty, self.reason)
            }
            TypeErrorKind::NotOneOf { ty, options } => {
                write!(
                    f,
                    "type {} is not one of {:?} ({})",
                    ty, options, self.reason
                )
            }
            TypeErrorKind::ArityMismatch { expected, found } => {
                write!(
                    f,
                    "wrong number of arguments: expected {}, found {} ({})",
                    expected, found, self.reason
                )
            }
            TypeErrorKind::NotCallable { ty } => {
                write!(f, "type {} is not callable ({})", ty, self.reason)
            }
            TypeErrorKind::UndefinedVariable { name } => {
                write!(f, "undefined variable: {}", name)
            }
            TypeErrorKind::UndefinedFunction { name } => {
                write!(f, "undefined function: {}", name)
            }
            TypeErrorKind::MemberAccess { message } => write!(f, "{message}"),
            TypeErrorKind::RecursionLimit => {
                write!(f, "type inference recursion limit exceeded")
            }
            TypeErrorKind::AssignToImmutable { name, .. } => {
                write!(f, "cannot assign to immutable variable `{}`", name)
            }
            TypeErrorKind::AssignToLoopVariable { name } => {
                write!(f, "cannot assign to loop variable `{}`", name)
            }
        }
    }
}

impl std::error::Error for TypeError {}

impl TypeError {
    pub fn mismatch(
        expected: InferType,
        found: InferType,
        span: Span,
        reason: ConstraintReason,
    ) -> Self {
        TypeError {
            kind: TypeErrorKind::Mismatch { expected, found },
            span,
            reason,
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn infinite_type(
        var: TypeVarId,
        ty: InferType,
        span: Span,
        reason: ConstraintReason,
    ) -> Self {
        TypeError {
            kind: TypeErrorKind::InfiniteType { var, ty },
            span,
            reason,
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn not_one_of(
        ty: InferType,
        options: Vec<InferType>,
        span: Span,
        reason: ConstraintReason,
    ) -> Self {
        TypeError {
            kind: TypeErrorKind::NotOneOf { ty, options },
            span,
            reason,
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn arity_mismatch(
        expected: usize,
        found: usize,
        span: Span,
        reason: ConstraintReason,
    ) -> Self {
        TypeError {
            kind: TypeErrorKind::ArityMismatch { expected, found },
            span,
            reason,
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn not_callable(ty: InferType, span: Span, reason: ConstraintReason) -> Self {
        TypeError {
            kind: TypeErrorKind::NotCallable { ty },
            span,
            reason,
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn undefined_variable(name: String, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::UndefinedVariable { name },
            span,
            reason: ConstraintReason::Other("variable lookup".to_string()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn undefined_function(name: String, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::UndefinedFunction { name },
            span,
            reason: ConstraintReason::Other("function call".to_string()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn member_access(message: String, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::MemberAccess { message },
            span,
            reason: ConstraintReason::Other("member access".to_string()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn recursion_limit(span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::RecursionLimit,
            span,
            reason: ConstraintReason::Other("recursion limit".to_string()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn assign_to_immutable(
        name: String,
        span: Span,
        binding_span: Option<Span>,
        suggestion: Option<TypeErrorSuggestion>,
    ) -> Self {
        TypeError {
            kind: TypeErrorKind::AssignToImmutable {
                name: name.clone(),
                binding_span,
            },
            span,
            reason: ConstraintReason::Assignment { var_name: name },
            secondary_spans: Vec::new(),
            help: Some("make the binding mutable: `let mut`".to_string()),
            suggestion,
        }
    }

    pub fn assign_to_loop_variable(name: String, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::AssignToLoopVariable { name: name.clone() },
            span,
            reason: ConstraintReason::Assignment { var_name: name },
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }
}

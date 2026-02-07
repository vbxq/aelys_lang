use super::ConstraintReason;
use crate::types::InferType;
use aelys_syntax::Span;

/// A type constraint that must be satisfied
#[derive(Debug, Clone)]
pub enum Constraint {
    /// Two types must be equal: τ₁ = τ₂
    Equal {
        left: InferType,
        right: InferType,
        span: Span,
        /// Description for error messages
        reason: ConstraintReason,
    },

    /// Type must be one of the given options (e.g., for + operator)
    OneOf {
        ty: InferType,
        options: Vec<InferType>,
        span: Span,
        reason: ConstraintReason,
    },
}

impl Constraint {
    /// Create an equality constraint
    pub fn equal(left: InferType, right: InferType, span: Span, reason: ConstraintReason) -> Self {
        Constraint::Equal {
            left,
            right,
            span,
            reason,
        }
    }

    /// Create a OneOf constraint
    pub fn one_of(
        ty: InferType,
        options: Vec<InferType>,
        span: Span,
        reason: ConstraintReason,
    ) -> Self {
        Constraint::OneOf {
            ty,
            options,
            span,
            reason,
        }
    }

    /// Get the span of this constraint
    pub fn span(&self) -> Span {
        match self {
            Constraint::Equal { span, .. } => *span,
            Constraint::OneOf { span, .. } => *span,
        }
    }

    /// Get the reason for this constraint
    pub fn reason(&self) -> &ConstraintReason {
        match self {
            Constraint::Equal { reason, .. } => reason,
            Constraint::OneOf { reason, .. } => reason,
        }
    }
}

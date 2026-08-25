use super::ConstraintReason;
use crate::types::InferType;
use crate::unify::Dir;
use aelys_syntax::Span;

#[derive(Debug, Clone)]
pub enum Constraint {
    Equal {
        left: InferType,
        right: InferType,
        span: Span,
        reason: ConstraintReason,
        dir: Dir,
    },

    OneOf {
        ty: InferType,
        options: Vec<InferType>,
        span: Span,
        reason: ConstraintReason,
    },
}

impl Constraint {
    pub fn equal(left: InferType, right: InferType, span: Span, reason: ConstraintReason) -> Self {
        Constraint::Equal {
            left,
            right,
            span,
            reason,
            dir: Dir::Exact,
        }
    }

    pub fn flows(
        found: InferType,
        required: InferType,
        span: Span,
        reason: ConstraintReason,
    ) -> Self {
        Constraint::Equal {
            left: found,
            right: required,
            span,
            reason,
            dir: Dir::Flow,
        }
    }

    pub fn flows_into(
        required: InferType,
        found: InferType,
        span: Span,
        reason: ConstraintReason,
    ) -> Self {
        Constraint::Equal {
            left: required,
            right: found,
            span,
            reason,
            dir: Dir::FlowRev,
        }
    }

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

    pub fn span(&self) -> Span {
        match self {
            Constraint::Equal { span, .. } => *span,
            Constraint::OneOf { span, .. } => *span,
        }
    }

    pub fn reason(&self) -> &ConstraintReason {
        match self {
            Constraint::Equal { reason, .. } => reason,
            Constraint::OneOf { reason, .. } => reason,
        }
    }
}


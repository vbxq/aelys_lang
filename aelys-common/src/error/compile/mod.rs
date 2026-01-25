use aelys_syntax::{Source, Span};
use std::sync::Arc;

mod annotation;
mod code;
mod format;
mod kind;
mod message;

pub use kind::CompileErrorKind;

#[derive(Debug)]
pub struct CompileError {
    pub kind: CompileErrorKind,
    pub span: Span,
    pub source: Arc<Source>,
}

impl CompileError {
    pub fn new(kind: CompileErrorKind, span: Span, source: Arc<Source>) -> Self {
        Self { kind, span, source }
    }
}

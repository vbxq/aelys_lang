use aelys_syntax::{Source, Span};
use std::sync::Arc;

mod annotation;
mod code;
mod format;
mod kind;
mod message;

use crate::diagnostic::{Diagnostic, Severity};

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

    pub fn to_diagnostic(&self) -> Diagnostic {
        let mut diag = Diagnostic::new(Severity::Error, self.kind.message())
            .with_code(format!("E{:04}", self.kind.code()));

        let annotation = self.kind.annotation().trim();
        let primary_msg = if annotation.is_empty() {
            None
        } else {
            Some(annotation.to_string())
        };
        diag = diag.with_primary_label(self.source.clone(), self.span, primary_msg);

        match &self.kind {
            CompileErrorKind::TypeInferenceError(raw) => {
                for line in raw.lines().skip(1) {
                    let detail = line.trim();
                    if !detail.is_empty() {
                        diag.add_note(detail.to_string());
                    }
                }
            }
            CompileErrorKind::BackendDiagnostic { note, help, .. } => {
                if let Some(note) = note {
                    diag.add_note(note.clone());
                }
                if let Some(help) = help {
                    diag.add_help(help.clone());
                }
            }
            CompileErrorKind::AssignToImmutable(_) => {
                diag.add_help("make the binding mutable: `let mut`".to_string());
            }
            CompileErrorKind::ModuleNotFound { searched_paths, .. } => {
                // FIXME: legacy VM code, clean up
                if !searched_paths.is_empty() {
                    diag.add_note(format!("searched in: {}", searched_paths.join(", ")));
                }
            }
            CompileErrorKind::SymbolNotPublic { module, .. } => {
                diag.add_help(format!(
                    "add 'pub' before the declaration in {}.aelys",
                    module
                ));
            }
            CompileErrorKind::StdlibNotAvailable { .. } => {
                // TODO: clean up when std is done
                diag.add_note("standard library will be available in a future version".to_string());
            }
            CompileErrorKind::SymbolConflict { .. } => {
                diag.add_help("use 'as' alias to disambiguate".to_string());
            }
            _ => {}
        }

        diag
    }
}

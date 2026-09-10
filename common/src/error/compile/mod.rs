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
            CompileErrorKind::SourceUnreadable { path, .. } => {
                diag.add_help(format!("check that `{}` exists and is readable", path));
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
                if !searched_paths.is_empty() {
                    diag.add_note(format!("searched in: {}", searched_paths.join(", ")));
                }
            }
            CompileErrorKind::AmbiguousModule { roots, .. } => {
                diag.add_note(format!("resolved in: {}", roots.join(", ")));
                diag.add_help(
                    "drop one of the `-I` roots, or delete the copy that duplicates it".to_string(),
                );
            }
            CompileErrorKind::SymbolNotPublic { module, .. } => {
                diag.add_help(format!(
                    "add `pub` before the declaration in module '{}'",
                    module
                ));
            }
            CompileErrorKind::ConflictingExternalSymbol { .. } => {
                diag.add_note("one symbol has one body: the linker keeps a single one".to_string());
                diag.add_help(
                    "rename the Aelys function, or drop the external declaration".to_string(),
                );
            }
            CompileErrorKind::ConflictingForeignDeclarations { .. } => {
                diag.add_note(
                    "one symbol has one signature: the declarations that name it have to say the \
                     same thing about it"
                        .to_string(),
                );
                diag.add_help(
                    "make the declarations agree, or keep a single one and import it".to_string(),
                );
            }
            CompileErrorKind::ReservedRuntimeSymbol { .. } => {
                diag.add_note(
                    "the runtime archive either defines this symbol or imports it from libc"
                        .to_string(),
                );
                diag.add_help("rename the function".to_string());
            }
            CompileErrorKind::MalformedForeignDecl { .. } => {
                diag.add_help(
                    "the only accepted form is `unsafe extern [nogc] fn NAME(PARAMS) [-> T]`, \
                     with no `pub`, no decorator, no type parameter and no body, at the top level"
                        .to_string(),
                );
            }
            CompileErrorKind::ForeignSignatureType { .. } => {
                diag.add_note(
                    "a foreign signature is an abi promise, so every type in it has to have a c \
                     meaning the compiler can hold"
                        .to_string(),
                );
                diag.add_help(
                    "the surface is the integers, `f32`, `f64`, `bool` and `&T`; `void` is \
                     accepted as a return type and nowhere else"
                        .to_string(),
                );
            }
            CompileErrorKind::LinkedLibraryClaimsRuntimeSymbol { .. } => {
                diag.add_note(
                    "the runtime's own calls would be resolved against the library's definition, \
                     which is a crash or a wrong answer rather than a link error"
                        .to_string(),
                );
                diag.add_help(
                    "drop the `-l` that carries this symbol, or link a build of it that does not \
                     define it"
                        .to_string(),
                );
            }
            CompileErrorKind::SymbolConflict { .. } => {
                diag.add_help("use 'as' to bind one of them to another name".to_string());
            }
            CompileErrorKind::DuplicateDefinition {
                name,
                previous_form,
                previous,
                ..
            } => {
                diag.add_secondary_label(
                    self.source.clone(),
                    *previous,
                    Some(format!(
                        "`{}` is first defined here, as {}",
                        name, previous_form
                    )),
                );
                diag.add_note(
                    "functions, globals, structs and enums share one top level namespace"
                        .to_string(),
                );
                diag.add_help("rename one of them".to_string());
            }
            CompileErrorKind::ForeignHeaderImport { .. } => {
                diag.add_note(
                    "C headers are reached by the same `needs` keyword, but nothing reads them yet"
                        .to_string(),
                );
                diag.add_help(
                    "declare the functions you need by hand with `unsafe extern fn NAME(...) -> T`; \
                     the parameter and return types must be integers, floats, `bool` or references, \
                     see E0615"
                        .to_string(),
                );
            }
            CompileErrorKind::WildcardImport { module_path } => {
                diag.add_help(format!(
                    "name what you need: `needs <symbol> from {}`",
                    module_path
                ));
            }
            _ => {}
        }

        diag
    }
}

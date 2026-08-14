use aelys_codegen::{AirNodeLocation, AirNodePosition, LlvmBackendError};
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_common::{Diagnostic, Replacement, Severity, Suggestion};
use aelys_sema::{TypeError, TypeErrorKind};
use aelys_syntax::{Source, Span as SyntaxSpan};
use std::path::Path;
use std::sync::Arc;

pub(super) fn load_source_for_diagnostics(path: &Path) -> Arc<Source> {
    let name = path.display().to_string();
    match std::fs::read_to_string(path) {
        Ok(content) => Source::new(name, content),
        Err(_) => Source::new(name, ""),
    }
}

pub(super) fn backend_diagnostic_error(
    source: Arc<Source>,
    span: SyntaxSpan,
    backend: &str,
    message: impl Into<String>,
    note: Option<String>,
    help: Option<String>,
) -> AelysError {
    AelysError::Compile(CompileError::new(
        CompileErrorKind::BackendDiagnostic {
            backend: backend.to_string(),
            message: message.into(),
            note,
            help,
        },
        span,
        source,
    ))
}

const VEC_SURFACE_ANNOTATION: &str = "Vec used outside the guaranteed value-semantics surface";

pub(super) fn mono_errors_to_error(
    errors: Vec<String>,
    span: SyntaxSpan,
    source: Arc<Source>,
) -> AelysError {
    let marker = aelys_air::passes::vec_surface::MARKER;
    let (surface, other): (Vec<String>, Vec<String>) =
        errors.into_iter().partition(|e| e.starts_with(marker));

    if surface.is_empty() {
        return backend_diagnostic_error(
            source.clone(),
            span,
            "monomorphization",
            numbered(&other),
            None,
            None,
        );
    }

    let mut diagnostics: Vec<Diagnostic> = surface
        .iter()
        .map(|message| {
            Diagnostic::new(Severity::Error, message)
                .with_code("E0412")
                .with_primary_label(
                    source.clone(),
                    span,
                    Some(VEC_SURFACE_ANNOTATION.to_string()),
                )
        })
        .collect();
    if !other.is_empty() {
        diagnostics.push(
            Diagnostic::new(
                Severity::Error,
                &format!("[monomorphization] {}", numbered(&other)),
            )
            .with_code("E0901")
            .with_primary_label(
                source.clone(),
                span,
                Some("backend error".to_string()),
            ),
        );
    }
    AelysError::Multiple(diagnostics)
}

pub(super) fn vec_surface_errors_to_error(
    errors: Vec<aelys_air::passes::vec_surface::VecSurfaceError>,
    air: &aelys_air::AirProgram,
    source: Arc<Source>,
) -> AelysError {
    let diagnostics: Vec<Diagnostic> = errors
        .iter()
        .map(|err| {
            let span = err
                .span
                .map(|s| air_span_to_syntax_span(s, source.as_ref()))
                .unwrap_or_else(|| program_anchor_span(air, source.as_ref()));
            Diagnostic::new(Severity::Error, &err.message)
                .with_code("E0412")
                .with_primary_label(
                    source.clone(),
                    span,
                    Some(VEC_SURFACE_ANNOTATION.to_string()),
                )
        })
        .collect();
    AelysError::Multiple(diagnostics)
}

fn numbered(errors: &[String]) -> String {
    errors
        .iter()
        .enumerate()
        .map(|(i, e)| format!("{}. {}", i + 1, e))
        .collect::<Vec<_>>()
        .join("\n")
}

// the emitted set is a faithful 1:1 of the borrow checker's error set.
pub(super) fn bir_diagnostics_to_error(
    diags: Vec<aelys_air::bir::BirDiagnostic>,
    source: Arc<Source>,
) -> AelysError {
    let diagnostics: Vec<Diagnostic> = diags
        .into_iter()
        .map(|d| {
            let clamp = d.code == "E0727";
            let cut = |span| {
                if clamp {
                    first_line_only(source.as_ref(), span)
                } else {
                    span
                }
            };
            let hint = d.hint.clone().unwrap_or_else(|| primary_hint(d.marker));
            let mut diag = Diagnostic::new(Severity::Error, &d.primary.1)
                .with_code(d.code)
                .with_primary_label(source.clone(), cut(d.primary.0), Some(hint));
            for (span, label) in &d.secondaries {
                diag.add_secondary_label(source.clone(), cut(*span), Some(label.clone()));
            }
            if let Some(note) = &d.note {
                diag.add_note(note.clone());
            }
            if let Some(help) = &d.help {
                diag.add_help(help.clone());
            }
            diag
        })
        .collect();

    if diagnostics.is_empty() {
        backend_diagnostic_error(
            source.clone(),
            fallback_source_span(source.as_ref()),
            "borrow-check",
            "borrow check failed",
            None,
            None,
        )
    } else {
        AelysError::Multiple(diagnostics)
    }
}

fn first_line_only(source: &Source, span: SyntaxSpan) -> SyntaxSpan {
    let bytes = source.content.as_bytes();
    let end = span.end.min(bytes.len());
    let start = span.start.min(end);
    match bytes[start..end].iter().position(|b| *b == b'\n') {
        Some(i) => SyntaxSpan::new(span.start, start + i, span.line, span.column),
        None => span,
    }
}

fn primary_hint(marker: &str) -> String {
    match marker {
        "[borrow]" => "borrow occurs here".to_string(),
        "[move]" => "move occurs here".to_string(),
        "[escape]" => "borrow escapes here".to_string(),
        "[nogc]" => "managed memory reached here".to_string(),
        _ => "here".to_string(),
    }
}

pub(super) fn sema_errors_to_diagnostics(
    errors: Vec<TypeError>,
    source: Arc<Source>,
) -> AelysError {
    let mut sorted_errors = errors;
    sorted_errors.sort_by(|a, b| {
        let left = (a.span.start, a.span.end, a.span.line, a.span.column);
        let right = (b.span.start, b.span.end, b.span.line, b.span.column);
        left.cmp(&right)
            .then_with(|| a.to_string().cmp(&b.to_string()))
    });
    sorted_errors.dedup_by(|a, b| a.span == b.span && a.to_string() == b.to_string());

    let diagnostics: Vec<Diagnostic> = sorted_errors
        .into_iter()
        .map(|err| type_error_to_diagnostic(&err, &source))
        .collect();

    if diagnostics.is_empty() {
        AelysError::Compile(CompileError::new(
            CompileErrorKind::TypeInferenceError("unknown type error".to_string()),
            fallback_source_span(source.as_ref()),
            source,
        ))
    } else {
        AelysError::Multiple(diagnostics)
    }
}

fn type_error_to_diagnostic(error: &TypeError, source: &Arc<Source>) -> Diagnostic {
    let (code, message, annotation) = match &error.kind {
        TypeErrorKind::Mismatch { expected, found } => (
            "E0301",
            format!("expected `{}`, found `{}`", expected, found),
            format!("expected `{}`, found `{}`", expected, found),
        ),
        TypeErrorKind::InfiniteType { var, ty } => (
            "E0305",
            format!("infinite type: {} = {}", var, ty),
            "infinite type".to_string(),
        ),
        TypeErrorKind::NotOneOf { ty, options } => {
            let opts: Vec<_> = options.iter().map(|o| format!("`{}`", o)).collect();
            (
                "E0301",
                format!("type `{}` is not one of [{}]", ty, opts.join(", ")),
                "type mismatch".to_string(),
            )
        }
        TypeErrorKind::ArityMismatch { expected, found } => {
            let reason_str = error.reason.to_string();
            (
                "E0302",
                format!(
                    "this function takes {} argument{} but {} {} supplied ({})",
                    expected,
                    if *expected == 1 { "" } else { "s" },
                    found,
                    if *found == 1 { "was" } else { "were" },
                    reason_str,
                ),
                "wrong number of arguments".to_string(),
            )
        }
        TypeErrorKind::NotCallable { ty } => (
            "E0303",
            format!("type `{}` is not callable", ty),
            "not callable".to_string(),
        ),
        TypeErrorKind::UndefinedVariable { name } => (
            "E0201",
            format!("undefined variable `{}`", name),
            "not found in this scope".to_string(),
        ),
        TypeErrorKind::UndefinedFunction { name } => (
            "E0203",
            format!("undefined function `{}`", name),
            "not found in this scope".to_string(),
        ),
        TypeErrorKind::MemberAccess { message } => (
            "E0304",
            message.clone(),
            "invalid member access".to_string(),
        ),
        TypeErrorKind::RecursionLimit => (
            "E0309",
            "type inference recursion limit exceeded".to_string(),
            "recursion limit".to_string(),
        ),
        TypeErrorKind::AssignToImmutable { name, .. } => (
            "E0401",
            format!("cannot assign to immutable variable `{}`", name),
            "assignment to immutable variable".to_string(),
        ),
        TypeErrorKind::AssignToLoopVariable { name } => (
            "E0402",
            format!("cannot assign to loop variable `{}`", name),
            "controlled by the for loop".to_string(),
        ),
        TypeErrorKind::RcOutOfSurface { detail } => (
            "E0410",
            format!("[rc-stage1] {}", detail),
            "Rc used outside the Stage 1 supported surface".to_string(),
        ),
        TypeErrorKind::VecOutOfSurface { detail } => (
            "E0412",
            format!("[vec-surface] {}", detail),
            "Vec used outside the guaranteed value-semantics surface".to_string(),
        ),
        TypeErrorKind::VecForeachUnsupported => (
            "E0414",
            error.to_string(),
            "iterating a `Vec<T>` with `for` is not supported yet".to_string(),
        ),
        TypeErrorKind::MutIndexRefUnsupported => (
            "E0415",
            error.to_string(),
            "a mutable reference through an element or field projection is not supported yet"
                .to_string(),
        ),
        TypeErrorKind::MutRefImmutableBinding { .. } => (
            "E0417",
            error.to_string(),
            "mutable reference to an immutable binding".to_string(),
        ),
        TypeErrorKind::NestedFnShadowsOuter { .. } => (
            "E0418",
            error.to_string(),
            "nested function shadows an outer function".to_string(),
        ),
        TypeErrorKind::ReservedTypeName { .. } => (
            "E0419",
            error.to_string(),
            "reserved builtin type name".to_string(),
        ),
        TypeErrorKind::RcFieldAssignIndirect => (
            "E0420",
            error.to_string(),
            "indirect right-hand side of an `Rc` field assignment".to_string(),
        ),
        TypeErrorKind::NoPlace { .. } => (
            "E0421",
            error.to_string(),
            "this expression denotes no storage".to_string(),
        ),
        TypeErrorKind::SharedMut { .. } => (
            "E0422",
            error.to_string(),
            "mutation through a shared reference".to_string(),
        ),
        TypeErrorKind::ClosureRefUnchecked { .. } => (
            "E0423",
            error.to_string(),
            "unchecked reference inside a closure body".to_string(),
        ),
        TypeErrorKind::GlobalBorrow { .. } => (
            "E0424",
            error.to_string(),
            "reference to module-level storage".to_string(),
        ),
        TypeErrorKind::SliceFormUnsupported { .. } => (
            "E0425",
            error.to_string(),
            "this slice form is not supported yet".to_string(),
        ),
        TypeErrorKind::MustUse { .. } => {
            ("E0411", error.to_string(), "unused `Result`".to_string())
        }
        TypeErrorKind::NogcOutOfPosition { .. } => (
            "E0728",
            error.to_string(),
            "`nogc fn` type out of position".to_string(),
        ),
        TypeErrorKind::NogcMutParam { .. } => (
            "E0728",
            error.to_string(),
            "`nogc fn` parameter declared `mut`".to_string(),
        ),
        TypeErrorKind::NogcParamShadowed { .. } => (
            "E0728",
            error.to_string(),
            "shadows a `nogc fn` parameter".to_string(),
        ),
        TypeErrorKind::NogcCallbackMismatch { .. } => {
            ("E0729", error.to_string(), "not a `nogc fn`".to_string())
        }
        TypeErrorKind::NogcBoundViolation { .. } => (
            "E0730",
            error.to_string(),
            "`nogc` bound not satisfied here".to_string(),
        ),
        TypeErrorKind::NogcBoundUnresolved { .. } => (
            "E0730",
            error.to_string(),
            "`nogc` bound cannot be proven here".to_string(),
        ),
        TypeErrorKind::NogcBoundGenericStruct { .. } => (
            "E0730",
            error.to_string(),
            "a generic struct cannot satisfy the `nogc` bound".to_string(),
        ),
        TypeErrorKind::NogcGenericAsValue { .. } => (
            "E0731",
            error.to_string(),
            "`nogc` generic used as a value".to_string(),
        ),
    };

    let clamp = matches!(code, "E0418" | "E0728" | "E0729" | "E0730" | "E0731");
    let cut = |span| {
        if clamp {
            first_line_only(source.as_ref(), span)
        } else {
            span
        }
    };

    let mut diag = Diagnostic::new(Severity::Error, &message)
        .with_code(code)
        .with_primary_label(source.clone(), cut(error.span), Some(annotation));

    for (span, label) in &error.secondary_spans {
        diag.add_secondary_label(source.clone(), cut(*span), Some(label.clone()));
    }

    if let TypeErrorKind::AssignToImmutable {
        name, binding_span, ..
    } = &error.kind
    {
        if let Some(bs) = binding_span {
            diag.add_secondary_label(
                source.clone(),
                *bs,
                Some(format!("`{}` first bound here", name)),
            );
        }
    }

    match &error.kind {
        TypeErrorKind::Mismatch { .. } => {
            let reason_str = error.reason.to_string();
            if !reason_str.is_empty() {
                diag.add_note(reason_str);
            }
        }
        _ => {}
    }

    if let Some(help) = &error.help {
        diag.add_help(help.clone());
    }

    if let Some(suggestion) = &error.suggestion {
        diag.add_suggestion(Suggestion {
            message: suggestion.message.clone(),
            replacements: vec![Replacement {
                span: suggestion.span,
                new_text: suggestion.new_text.clone(),
                source: source.clone(),
            }],
        });
    }

    diag
}

pub(super) fn fallback_source_span(source: &Source) -> SyntaxSpan {
    let end = if source.content.is_empty() { 0 } else { 1 };
    SyntaxSpan::new(0, end, 1, 1)
}

pub(super) fn program_anchor_span(air: &aelys_air::AirProgram, source: &Source) -> SyntaxSpan {
    main_function_air_span(air)
        .or_else(|| air.functions.iter().find_map(|function| function.span))
        .map(|span| air_span_to_syntax_span(span, source))
        .unwrap_or_else(|| fallback_source_span(source))
}

fn main_function_air_span(air: &aelys_air::AirProgram) -> Option<aelys_air::Span> {
    air.functions
        .iter()
        .find(|function| !function.is_extern && function.name == "main")
        .and_then(|function| function.span)
}

fn air_span_to_syntax_span(span: aelys_air::Span, source: &Source) -> SyntaxSpan {
    let len = source.content.len();
    let mut start = span.lo as usize;
    let mut end = span.hi as usize;

    if start > len {
        start = len;
    }
    if end > len {
        end = len;
    }
    if end < start {
        end = start;
    }
    if end == start && start < len {
        end += 1;
    }

    let (line, column) = source.line_col_at_offset(start);
    SyntaxSpan::new(start, end, line, column)
}

pub(super) fn llvm_backend_error_to_diagnostic(
    err: LlvmBackendError,
    air: &aelys_air::AirProgram,
    source: Arc<Source>,
) -> AelysError {
    let (message, note, help, location) = match err {
        LlvmBackendError::LlvmError(message) => {
            (format!("llvm error: {message}"), None, None, None)
        }
        LlvmBackendError::UnsupportedType(message) => {
            (format!("unsupported type: {message}"), None, None, None)
        }
        LlvmBackendError::UnsupportedInstruction(message) => (
            format!("unsupported instruction: {message}"),
            None,
            None,
            None,
        ),
        LlvmBackendError::InvalidNativeEntry(message) => (
            format!("invalid native entry: {message}"),
            None,
            native_entry_help(&message),
            None,
        ),
        LlvmBackendError::UnsupportedAir {
            kind,
            detail,
            location,
        } => {
            let mut note_parts = Vec::new();
            if !detail.is_empty() {
                note_parts.push(format!("reason: {detail}"));
            }
            if let Some(loc) = location.as_ref() {
                note_parts.push(format!("location: {}", format_air_location(loc)));
            }
            let note = if note_parts.is_empty() {
                None
            } else {
                Some(note_parts.join("; "))
            };
            (format!("unsupported AIR: {kind}"), note, None, location)
        }
    };

    let span = location
        .as_ref()
        .and_then(|loc| air_location_span(air, loc))
        .or_else(|| main_function_air_span(air))
        .map(|span| air_span_to_syntax_span(span, source.as_ref()))
        .unwrap_or_else(|| fallback_source_span(source.as_ref()));

    backend_diagnostic_error(source, span, "llvm-backend", message, note, help)
}

fn air_location_span(
    air: &aelys_air::AirProgram,
    location: &AirNodeLocation,
) -> Option<aelys_air::Span> {
    let function = air
        .functions
        .iter()
        .find(|function| function.name == location.function)?;

    if let Some(block_id) = location.block {
        let block = function
            .blocks
            .iter()
            .find(|block| block.id.0 == block_id)?;
        return match location.position {
            AirNodePosition::Stmt(index) => block
                .stmts
                .get(index)
                .and_then(|stmt| stmt.span)
                .or(function.span),
            AirNodePosition::Terminator => terminator_span(&block.terminator).or(function.span),
        };
    }

    function.span
}

fn terminator_span(terminator: &aelys_air::AirTerminator) -> Option<aelys_air::Span> {
    match terminator {
        aelys_air::AirTerminator::Panic { span, .. } => *span,
        _ => None,
    }
}

fn format_air_location(location: &AirNodeLocation) -> String {
    let mut rendered = format!("fn `{}`", location.function);
    if let Some(block) = location.block {
        rendered.push_str(&format!(", bb{block}"));
    }
    match location.position {
        AirNodePosition::Stmt(index) => rendered.push_str(&format!(", stmt #{index}")),
        AirNodePosition::Terminator => rendered.push_str(", terminator"),
    }
    rendered
}

fn native_entry_help(message: &str) -> Option<String> {
    if message.contains("main must have no parameters") {
        return Some("use `fn main()` or `fn main() -> i64`".to_string());
    }
    if message.contains("main return type must be void or i64") {
        return Some("change `main` return type to `void` or `i64`".to_string());
    }
    None
}

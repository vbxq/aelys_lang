
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

pub(super) fn sema_errors_to_diagnostics(errors: Vec<TypeError>, source: Arc<Source>) -> AelysError {
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
    };

    let mut diag = Diagnostic::new(Severity::Error, &message)
        .with_code(code)
        .with_primary_label(source.clone(), error.span, Some(annotation));

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

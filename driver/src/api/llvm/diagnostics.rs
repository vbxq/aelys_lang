use aelys_codegen::{AirNodeLocation, AirNodePosition, LlvmBackendError};
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_common::{Diagnostic, Replacement, Severity, Suggestion};
use aelys_sema::{TypeError, TypeErrorKind};
use aelys_syntax::{Source, Span as SyntaxSpan};
use std::sync::Arc;

// the qualification head is unspellable in source, so it may never reach a user
fn user_facing(message: impl Into<String>) -> String {
    message.into().replace(aelys_sema::modules::TYPE_HEAD, "")
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
            message: user_facing(message),
            note,
            help,
        },
        span,
        source,
    ))
}

pub(super) fn multiple_diagnostics(errors: Vec<AelysError>) -> AelysError {
    AelysError::Multiple(
        errors
            .into_iter()
            .flat_map(|error| error.to_diagnostics())
            .collect(),
    )
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
            Diagnostic::new(Severity::Error, &user_facing(message.as_str()))
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
                &user_facing(format!("[monomorphization] {}", numbered(&other))),
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
            Diagnostic::new(Severity::Error, &user_facing(err.message.as_str()))
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

pub(super) fn duplicate_symbol_errors_to_error(
    duplicates: Vec<aelys_air::symbols::DuplicateSymbol>,
    typed: &aelys_sema::TypedProgram,
    air: &aelys_air::AirProgram,
    source: Arc<Source>,
) -> AelysError {
    let sites = aelys_air::symbols::decl_sites_by_symbol(typed);
    let anchor = program_anchor_span(air, source.as_ref());
    let diagnostics: Vec<Diagnostic> = duplicates
        .iter()
        .map(|dup| {
            let declared = sites.get(&dup.symbol).map(Vec::as_slice).unwrap_or(&[]);
            let site = |i: usize| {
                declared.get(i).map(|site| site.span).or_else(|| {
                    dup.spans
                        .get(i)
                        .copied()
                        .flatten()
                        .map(|s| air_span_to_syntax_span(s, source.as_ref()))
                })
            };
            let label = |i: usize| {
                let span = site(i).unwrap_or(anchor);
                let hint = match declared.get(i).and_then(|site| site.parent.as_deref()) {
                    Some(parent) => format!("defined here, inside `{}`", parent),
                    None => "defined here".to_string(),
                };
                (first_line_only(source.as_ref(), span), hint)
            };
            let (first_span, first_hint) = label(0);
            let (second_span, second_hint) = label(1);
            if dup.has_extern {
                let mut diag = CompileError::new(
                    CompileErrorKind::ConflictingExternalSymbol {
                        symbol: dup.symbol.clone(),
                    },
                    first_span,
                    source.clone(),
                )
                .to_diagnostic();
                let second = site(1).map(|span| first_line_only(source.as_ref(), span));
                if let Some(second) = second.filter(|span| *span != first_span) {
                    diag.add_secondary_label(
                        source.clone(),
                        second,
                        Some("and claimed again here".to_string()),
                    );
                }
                return diag;
            }
            let message = format!(
                "[symbol] two functions compile to the same symbol `{}`, so a call to one would \
                 reach the other",
                dup.symbol
            );
            let mut diag = Diagnostic::new(Severity::Error, &message)
                .with_code("E0427")
                .with_primary_label(source.clone(), first_span, Some(first_hint));
            if second_span != first_span {
                diag.add_secondary_label(source.clone(), second_span, Some(second_hint));
            }
            let help = if dup.symbol.starts_with("__mono_") {
                "rename one of them; a generic instance is named from the function name and its \
                 type arguments joined by `_`, so two different pairs can produce one name"
            } else {
                "rename one of them; nested functions do not get separate symbols yet"
            };
            diag.add_help(help.to_string());
            diag
        })
        .collect();
    AelysError::Multiple(diagnostics)
}

pub(super) fn reserved_name_errors_to_error(
    names: Vec<aelys_air::symbols::ReservedUserName>,
    source: Arc<Source>,
) -> AelysError {
    let diagnostics: Vec<Diagnostic> = names
        .iter()
        .map(|reserved| {
            let message = format!(
                "[reserved-symbol] `{}` starts with `{}`, which is reserved for compiler- and \
                 runtime-generated symbols",
                reserved.name,
                aelys_air::symbols::RESERVED_PREFIX
            );
            let mut diag = Diagnostic::new(Severity::Error, &message)
                .with_code("E0428")
                .with_primary_label(
                    source.clone(),
                    first_line_only(source.as_ref(), reserved.span),
                    Some("reserved symbol name".to_string()),
                );
            diag.add_help("rename the function".to_string());
            diag
        })
        .collect();
    AelysError::Multiple(diagnostics)
}

pub(super) fn foreign_clash_errors_to_error(
    clashes: Vec<aelys_air::bir::ForeignClash>,
    source: Arc<Source>,
) -> AelysError {
    let diagnostics: Vec<Diagnostic> = clashes
        .iter()
        .map(|clash| {
            CompileError::new(
                CompileErrorKind::ConflictingForeignDeclarations {
                    symbol: clash.name.clone(),
                    reason: clash.reason.clone(),
                },
                first_line_only(source.as_ref(), clash.span),
                source.clone(),
            )
            .to_diagnostic()
        })
        .collect();
    AelysError::Multiple(diagnostics)
}

pub(super) fn foreign_signature_errors_to_error(
    violations: Vec<aelys_air::symbols::ForeignSignatureViolation>,
    source: Arc<Source>,
) -> AelysError {
    let diagnostics: Vec<Diagnostic> = violations
        .iter()
        .map(|violation| {
            CompileError::new(
                CompileErrorKind::ForeignSignatureType {
                    function: violation.function.clone(),
                    what: violation.what.clone(),
                    spelling: violation.spelling.clone(),
                    reason: violation.reason.to_string(),
                },
                first_line_only(source.as_ref(), violation.span),
                source.clone(),
            )
            .to_diagnostic()
        })
        .collect();
    AelysError::Multiple(diagnostics)
}

pub(super) fn runtime_symbol_errors_to_error(
    claims: Vec<aelys_air::symbols::ReservedRuntimeSymbol>,
    air: &aelys_air::AirProgram,
    source: Arc<Source>,
) -> AelysError {
    let anchor = program_anchor_span(air, source.as_ref());
    let diagnostics: Vec<Diagnostic> = claims
        .iter()
        .map(|claim| {
            let span = claim
                .span
                .map(|s| air_span_to_syntax_span(s, source.as_ref()))
                .unwrap_or(anchor);
            CompileError::new(
                CompileErrorKind::ReservedRuntimeSymbol {
                    symbol: claim.symbol.clone(),
                },
                first_line_only(source.as_ref(), span),
                source.clone(),
            )
            .to_diagnostic()
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
            let mut diag = Diagnostic::new(Severity::Error, &user_facing(d.primary.1.as_str()))
                .with_code(d.code)
                .with_primary_label(source.clone(), cut(d.primary.0), Some(hint));
            for (span, label) in &d.secondaries {
                diag.add_secondary_label(
                    source.clone(),
                    cut(*span),
                    Some(user_facing(label.as_str())),
                );
            }
            if let Some(note) = &d.note {
                diag.add_note(user_facing(note.as_str()));
            }
            if let Some(help) = &d.help {
                diag.add_help(user_facing(help.as_str()));
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
        "[vec-cow]" => "write occurs here".to_string(),
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
        TypeErrorKind::RefMutability { found, required } => (
            "E0416",
            format!(
                "a shared borrow `{}` cannot be used where the mutable borrow `{}` is required",
                found, required
            ),
            format!("shared borrow `{}` here", found),
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
        TypeErrorKind::ForeignAsValue { .. } => (
            "E0616",
            error.to_string(),
            "external declaration used as a value".to_string(),
        ),
        TypeErrorKind::ForeignCallOutsideUnsafe { .. } => (
            "E0617",
            error.to_string(),
            "external call outside an `unsafe` block".to_string(),
        ),
        TypeErrorKind::ModuleItemNotPublic { .. } => {
            ("E0605", error.to_string(), "item is not public".to_string())
        }
        TypeErrorKind::ModuleItemNotFound { .. } => (
            "E0606",
            error.to_string(),
            "no such item in that module".to_string(),
        ),
        TypeErrorKind::FieldNotPublic { .. } => (
            "E0610",
            error.to_string(),
            "field is not public".to_string(),
        ),
        TypeErrorKind::PrivateTypeInPublicApi { .. } => (
            "E0611",
            error.to_string(),
            "private type in a public signature".to_string(),
        ),
    };

    let clamp = matches!(
        code,
        "E0418" | "E0611" | "E0728" | "E0729" | "E0730" | "E0731"
    );
    let cut = |span| {
        if clamp {
            first_line_only(source.as_ref(), span)
        } else {
            span
        }
    };

    let mut diag = Diagnostic::new(Severity::Error, &user_facing(message))
        .with_code(code)
        .with_primary_label(
            source.clone(),
            cut(error.span),
            Some(user_facing(annotation)),
        );

    for (span, label) in &error.secondary_spans {
        diag.add_secondary_label(
            source.clone(),
            cut(*span),
            Some(user_facing(label.as_str())),
        );
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
                diag.add_note(user_facing(reason_str));
            }
        }
        _ => {}
    }

    if let Some(help) = &error.help {
        diag.add_help(user_facing(help.as_str()));
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

#[cfg(test)]
mod tests {
    use super::*;
    use aelys_air::symbols::DuplicateSymbol;

    fn stack(src: &str) -> (aelys_sema::TypedProgram, aelys_air::AirProgram, Arc<Source>) {
        let source = Source::new("<unit>", src);
        let typed = super::super::compile_to_typed_ast(src).expect("fixture must type-check");
        let air = match aelys_air::lower::try_lower(&typed) {
            Ok(air) => air,
            Err(_) => panic!("fixture must lower"),
        };
        (typed, air, source)
    }

    fn render(dup: DuplicateSymbol, src: &str) -> String {
        let (typed, air, source) = stack(src);
        duplicate_symbol_errors_to_error(vec![dup], &typed, &air, source).to_string()
    }

    fn label_lines(rendered: &str) -> Vec<(usize, char)> {
        let mut found = Vec::new();
        let mut current = 0usize;
        for line in rendered.lines() {
            let Some((gutter, rest)) = line.split_once('|') else {
                continue;
            };
            match gutter.trim().parse::<usize>() {
                Ok(number) => current = number,
                Err(_) => match rest.trim_start().chars().next() {
                    Some(marker @ ('^' | '-')) => found.push((current, marker)),
                    _ => {}
                },
            }
        }
        found
    }

    #[test]
    fn duplicate_symbol_renders_with_no_matching_typed_declaration() {
        let rendered = render(
            DuplicateSymbol {
                symbol: "__mono_ghost_i64".to_string(),
                spans: vec![None, None],
                has_extern: false,
            },
            "fn main() -> i64 { return 0 }\n",
        );
        assert!(
            rendered.contains("E0427") && rendered.contains("__mono_ghost_i64"),
            "the zero-declaration fallback must still render a diagnostic, got:\n{rendered}"
        );
        assert_eq!(
            label_lines(&rendered),
            vec![(1, '^')],
            "both labels fall to the program anchor, so exactly one is drawn, got:\n{rendered}"
        );
        assert!(
            rendered.contains("type arguments joined by"),
            "a mangled symbol must not be explained as a nested function, got:\n{rendered}"
        );
    }

    #[test]
    fn conflicting_external_symbol_leaves_an_unplaced_second_site_undrawn() {
        let rendered = render(
            DuplicateSymbol {
                symbol: "solo".to_string(),
                spans: vec![None, None],
                has_extern: true,
            },
            "fn solo() -> i64 { return 1 }\nfn main() -> i64 { return solo() }\n",
        );
        assert!(
            rendered.contains("E0612") && rendered.contains("solo"),
            "the extern route must still render, got:\n{rendered}"
        );
        assert_eq!(
            label_lines(&rendered),
            vec![(1, '^')],
            "the declaration draws the primary and nothing draws on `fn main`, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("and claimed again here"),
            "an unplaced second site must not be announced, got:\n{rendered}"
        );
    }

    #[test]
    fn duplicate_symbol_renders_with_one_matching_typed_declaration() {
        let rendered = render(
            DuplicateSymbol {
                symbol: "solo".to_string(),
                spans: vec![None, None],
                has_extern: false,
            },
            "fn solo() -> i64 { return 1 }\nfn main() -> i64 { return solo() }\n",
        );
        assert!(
            rendered.contains("E0427") && rendered.contains("solo"),
            "the one-declaration fallback must still render a diagnostic, got:\n{rendered}"
        );
        assert_eq!(
            label_lines(&rendered),
            vec![(1, '^'), (2, '-')],
            "the declared site draws the primary and the anchor draws the secondary, got:\n{rendered}"
        );
        assert!(
            rendered.contains("nested functions do not get separate symbols yet"),
            "a bare symbol keeps the nested-function help, got:\n{rendered}"
        );
    }

    #[test]
    fn duplicate_symbol_renders_both_labels_from_the_air_spans() {
        let src = "fn one() -> i64 { return 1 }\nfn two() -> i64 { return 2 }\nfn main() -> i64 { return one() + two() }\n";
        let (typed, air, source) = stack(src);
        let spans: Vec<Option<aelys_air::Span>> = air
            .functions
            .iter()
            .filter(|function| function.name == "one" || function.name == "two")
            .map(|function| function.span)
            .collect();
        assert_eq!(spans.len(), 2, "the fixture must supply two air spans");
        let rendered = duplicate_symbol_errors_to_error(
            vec![DuplicateSymbol {
                symbol: "__mono_ghost_i64".to_string(),
                spans,
                has_extern: false,
            }],
            &typed,
            &air,
            source,
        )
        .to_string();
        assert!(
            rendered.contains("E0427") && rendered.contains("__mono_ghost_i64"),
            "the air-span fallback must render a diagnostic, got:\n{rendered}"
        );
        assert_eq!(
            label_lines(&rendered),
            vec![(1, '^'), (2, '-')],
            "with no typed declaration both labels come from the air spans, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("inside `"),
            "an air-span label has no parent to name, got:\n{rendered}"
        );
    }
}

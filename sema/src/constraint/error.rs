use super::ConstraintReason;
use crate::types::{InferType, TypeVarId};
use aelys_syntax::Span;
use std::fmt;

#[derive(Debug, Clone)]
pub struct TypeErrorSuggestion {
    pub message: String,
    pub span: Span,
    pub new_text: String,
}

#[derive(Debug, Clone)]
pub struct TypeError {
    pub kind: TypeErrorKind,
    pub span: Span,
    pub reason: ConstraintReason,
    pub secondary_spans: Vec<(Span, String)>,
    pub help: Option<String>,
    pub suggestion: Option<TypeErrorSuggestion>,
}

#[derive(Debug, Clone)]
pub enum TypeErrorKind {
    Mismatch {
        expected: InferType,
        found: InferType,
    },
    InfiniteType {
        var: TypeVarId,
        ty: InferType,
    },
    NotOneOf {
        ty: InferType,
        options: Vec<InferType>,
    },
    ArityMismatch {
        expected: usize,
        found: usize,
    },
    /// a shared borrow reached a position requiring an exclusive one
    RefMutability {
        found: InferType,
        required: InferType,
    },
    NotCallable {
        ty: InferType,
    },
    /// undefined variable
    UndefinedVariable {
        name: String,
    },
    /// undefined function
    UndefinedFunction {
        name: String,
    },
    MemberAccess {
        message: String,
    },
    RecursionLimit,
    AssignToImmutable {
        name: String,
        binding_span: Option<Span>,
    },
    AssignToLoopVariable {
        name: String,
    },
    RcOutOfSurface {
        detail: String,
    },
    VecOutOfSurface {
        detail: String,
    },
    VecForeachUnsupported,
    SliceFormUnsupported {
        detail: String,
    },
    MutIndexRefUnsupported,
    MutRefImmutableBinding {
        name: String,
    },
    // a nested fn reusing an outer fn's bare name clobbers its dispatch slot, a silent miscompile
    NestedFnShadowsOuter {
        name: String,
    },
    ReservedTypeName {
        name: String,
    },
    RcFieldAssignIndirect,
    NoPlace {
        what: String,
    },
    SharedMut {
        what: String,
        view: Option<InferType>,
    },
    ClosureRefUnchecked {
        what: String,
    },
    // bir has no local for a global, so two `&mut` of one global would genuinely alias
    GlobalBorrow {
        name: String,
        mutable: bool,
    },
    MustUse {
        error: InferType,
    },
    NogcOutOfPosition {
        detail: String,
    },
    NogcMutParam {
        detail: String,
    },
    NogcParamShadowed {
        detail: String,
    },
    NogcCallbackMismatch {
        detail: String,
    },
    NogcBoundViolation {
        detail: String,
    },
    // kind-keyed caret hint stays fail-closed instead of asserting a violation nobody proved
    NogcBoundUnresolved {
        detail: String,
    },
    NogcBoundGenericStruct {
        detail: String,
    },
    NogcGenericAsValue {
        detail: String,
    },
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
            TypeErrorKind::RefMutability { found, required } => {
                write!(
                    f,
                    "cannot use {} where {} is required ({})",
                    found, required, self.reason
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
            TypeErrorKind::RcOutOfSurface { detail } => {
                write!(f, "[rc-stage1] {}", detail)
            }
            TypeErrorKind::VecOutOfSurface { detail } => {
                write!(f, "[vec-surface] {}", detail)
            }
            TypeErrorKind::VecForeachUnsupported => write!(
                f,
                "[vec-foreach] iterating a `Vec<T>` with `for` is not supported yet. iterate an \
                 array (`[T; N]`) or a string instead, or index the `Vec` by hand with a counting \
                 `for i in 0..n` loop"
            ),
            TypeErrorKind::SliceFormUnsupported { detail } => write!(
                f,
                "[slice-form] {detail}. a slice is built from the address of element zero plus a \
                 length, so it needs a base that carries a length and a range that starts at 0"
            ),
            TypeErrorKind::NoPlace { what } => write!(
                f,
                "[no-place] {what} denotes no storage, so it has no address. bind it to a name \
                 first and use that binding"
            ),
            TypeErrorKind::SharedMut {
                what,
                view: Some(view),
            } => {
                let mutable = match view {
                    InferType::Slice { elem, .. } => InferType::Slice {
                        elem: elem.clone(),
                        mutable: true,
                    },
                    other => other.clone(),
                };
                write!(
                    f,
                    "[shared-mut] {what} writes through a shared `{view}`, which would break `mut \
                     XOR shared`. declare the slice `{mutable}`, or take it from a `mut` binding"
                )
            }
            TypeErrorKind::SharedMut { what, view: None } => write!(
                f,
                "[shared-mut] {what} writes through a shared `&`, which would break `mut XOR \
                 shared`. take the reference with `&mut` instead"
            ),
            TypeErrorKind::ClosureRefUnchecked { what } => write!(
                f,
                "[closure-unchecked] {what} inside a closure body; a lambda body carries no \
                 borrow-check, so the reference would be unchecked. form the reference outside \
                 the lambda and pass it as a parameter"
            ),
            TypeErrorKind::GlobalBorrow { name, mutable } => write!(
                f,
                "[global-borrow] cannot take `{}` of the module-level `{name}`; a global has no \
                 borrow-checked local, so two live references to it would alias unchecked. copy \
                 it into a local binding and reference that",
                if *mutable { "&mut" } else { "&" }
            ),
            TypeErrorKind::MutIndexRefUnsupported => write!(
                f,
                "[mut-index-ref] a mutable reference through an element or field projection is \
                 not supported yet; the refusal matches the shape of the operand and reads no \
                 type, so it covers every base alike. write `v[i] = x` or `p.f = x`, or take \
                 `&mut` of the whole binding"
            ),
            TypeErrorKind::MutRefImmutableBinding { name } => write!(
                f,
                "cannot take a mutable reference `&mut {name}` to immutable binding `{name}`; \
                 declare it with `let mut {name}`"
            ),
            TypeErrorKind::NestedFnShadowsOuter { name } => write!(
                f,
                "[nested-fn-shadow] a nested `fn {name}` reuses the name of an outer function; \
                 the shared namespace can bind a call to the wrong body. rename the nested \
                 function"
            ),
            TypeErrorKind::ReservedTypeName { name } => write!(
                f,
                "[reserved-type] `Vec` and `Rc` are reserved builtin type names; a user type named \
                 `{name}` would compete with intrinsic path resolution. rename the type"
            ),
            TypeErrorKind::RcFieldAssignIndirect => write!(
                f,
                "[rc-field-assign] the right-hand side of an `Rc` field assignment must be a direct \
                 producer (`Rc::new(...)`, `Rc::null()`, a bare identifier, an `Rc`-field read, or a \
                 call); an indirect form (`if`/`match`/a block/parentheses) leaves retain/release \
                 accounting unbalanced. bind the value to a name first"
            ),
            TypeErrorKind::MustUse { error } => write!(
                f,
                "[must-use] this `Result` can fail with `{error}`; \
                 use `?` to propagate, `match`/`catch` to handle, \
                 `.expect(\"…\")` to assert, or `discard` to intentionally ignore it"
            ),
            TypeErrorKind::NogcOutOfPosition { detail } => write!(f, "[nogc] {detail}"),
            TypeErrorKind::NogcMutParam { detail } => write!(f, "[nogc] {detail}"),
            TypeErrorKind::NogcParamShadowed { detail } => write!(f, "[nogc] {detail}"),
            TypeErrorKind::NogcCallbackMismatch { detail } => write!(f, "[nogc] {detail}"),
            TypeErrorKind::NogcBoundViolation { detail } => write!(f, "[nogc] {detail}"),
            TypeErrorKind::NogcBoundUnresolved { detail } => write!(f, "[nogc] {detail}"),
            TypeErrorKind::NogcBoundGenericStruct { detail } => write!(f, "[nogc] {detail}"),
            TypeErrorKind::NogcGenericAsValue { detail } => write!(f, "[nogc] {detail}"),
        }
    }
}

impl std::error::Error for TypeError {}

impl TypeError {
    pub fn with_secondary(mut self, span: Span, label: impl Into<String>) -> Self {
        self.secondary_spans.push((span, label.into()));
        self
    }

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

    pub fn rc_out_of_surface(detail: impl Into<String>, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::RcOutOfSurface {
                detail: detail.into(),
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn vec_out_of_surface(detail: impl Into<String>, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::VecOutOfSurface {
                detail: detail.into(),
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn no_place(what: impl Into<String>, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::NoPlace { what: what.into() },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn shared_mut(what: impl Into<String>, span: Span) -> Self {
        Self::shared_mut_view(what, None, span)
    }

    pub fn shared_mut_view(what: impl Into<String>, view: Option<InferType>, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::SharedMut {
                what: what.into(),
                view,
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn closure_ref_unchecked(what: impl Into<String>, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::ClosureRefUnchecked { what: what.into() },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn global_borrow(name: impl Into<String>, mutable: bool, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::GlobalBorrow {
                name: name.into(),
                mutable,
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn slice_form_unsupported(detail: impl Into<String>, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::SliceFormUnsupported {
                detail: detail.into(),
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn vec_foreach_unsupported(span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::VecForeachUnsupported,
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn mut_index_ref_unsupported(span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::MutIndexRefUnsupported,
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn mut_ref_immutable_binding(name: String, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::MutRefImmutableBinding { name },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: Some("make the binding mutable: `let mut`".to_string()),
            suggestion: None,
        }
    }

    pub fn nested_fn_shadows_outer(name: String, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::NestedFnShadowsOuter { name },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn reserved_type_name(name: String, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::ReservedTypeName { name },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn rc_field_assign_indirect(span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::RcFieldAssignIndirect,
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn must_use(span: Span, error: InferType) -> Self {
        TypeError {
            kind: TypeErrorKind::MustUse { error },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: None,
            suggestion: None,
        }
    }

    pub fn nogc_out_of_position(span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::NogcOutOfPosition {
                detail: "a `nogc fn` type is only allowed as an immutable function parameter type"
                    .to_string(),
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: Some(
                "write it as a bare immutable parameter type, as in `fn apply(f: nogc fn(&i32))`, \
                 or drop `nogc` and use a plain `fn` type here"
                    .to_string(),
            ),
            suggestion: None,
        }
    }

    pub fn nogc_mut_param(name: &str, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::NogcMutParam {
                detail: format!(
                    "a `nogc fn` parameter `{name}` cannot be `mut`; reassigning it would defeat the nogc guarantee"
                ),
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: Some(format!(
                "drop `mut` from `{name}`; a `nogc fn` parameter must stay immutable so the call \
                 site can trust it"
            )),
            suggestion: None,
        }
    }

    pub fn nogc_param_shadowed(name: &str, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::NogcParamShadowed {
                detail: format!(
                    "a `nogc fn` parameter `{name}` cannot be shadowed by a `let` binding; that would defeat the nogc guarantee"
                ),
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: Some(format!(
                "give the binding a different name, or rename the `{name}` parameter"
            )),
            suggestion: None,
        }
    }

    pub fn nogc_callback_mismatch(found: &str, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::NogcCallbackMismatch {
                detail: format!(
                    "expected a `nogc fn` argument (a direct reference to a `nogc`-declared \
                     function or a `nogc fn` parameter), found {found}"
                ),
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: Some(
                "pass a `nogc`-declared function by name, or a `nogc fn` parameter of the \
                 enclosing function; a lambda or a `let` binding never qualifies"
                    .to_string(),
            ),
            suggestion: None,
        }
    }

    pub fn nogc_bound_violation(
        fn_name: &str,
        type_param: &str,
        found: &InferType,
        span: Span,
    ) -> Self {
        TypeError {
            kind: TypeErrorKind::NogcBoundViolation {
                detail: format!(
                    "type parameter `{type_param}` of `{fn_name}` is bound `nogc`, but this call \
                     instantiates it with `{found}`, which is not a nogc value"
                ),
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: Some(
                "a nogc value is a primitive, a fixed array, a reference, a `nogc fn`, or a \
                 non-generic struct/enum built only from those"
                    .to_string(),
            ),
            suggestion: None,
        }
    }

    // fail-closed, a binding the call site cannot pin down is a reject
    pub fn nogc_bound_unresolved(fn_name: &str, type_param: &str, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::NogcBoundUnresolved {
                detail: format!(
                    "type parameter `{type_param}` of `{fn_name}` is bound `nogc`, but this call \
                     does not pin it to a concrete type, so the bound cannot be proven"
                ),
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: Some(
                "pass an argument whose type fixes the type parameter to a concrete nogc value"
                    .to_string(),
            ),
            suggestion: None,
        }
    }

    pub fn nogc_bound_generic_struct_arg(
        fn_name: &str,
        type_param: &str,
        struct_name: &str,
        span: Span,
    ) -> Self {
        TypeError {
            kind: TypeErrorKind::NogcBoundGenericStruct {
                detail: format!(
                    "type parameter `{type_param}` of `{fn_name}` is bound `nogc`, but this call \
                     passes the generic struct `{struct_name}`, whose type arguments are not \
                     tracked, so it is never a nogc value"
                ),
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: Some(format!(
                "a generic struct cannot satisfy a `nogc` bound; pass a non-generic struct, or \
                 give `{struct_name}` a non-generic wrapper built only from nogc values"
            )),
            suggestion: None,
        }
    }

    // a nogc-bound generic referenced as a value escapes every checked call site
    pub fn nogc_generic_as_value(fn_name: &str, span: Span) -> Self {
        TypeError {
            kind: TypeErrorKind::NogcGenericAsValue {
                detail: format!(
                    "`{fn_name}` is a generic function with a `nogc` bound, so it may only be \
                     called directly, never used as a value"
                ),
            },
            span,
            reason: ConstraintReason::Other(String::new()),
            secondary_spans: Vec::new(),
            help: Some(
                "call it directly, or drop the `nogc` bound if the callee need not be nogc"
                    .to_string(),
            ),
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


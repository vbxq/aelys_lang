use super::TypeInference;
use crate::constraint::TypeError;
use crate::infer::impls::MethodLookup;
use crate::modules::{MethodEntry, source_type_name};
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Expr, ExprKind, Receiver, Span, Stmt, StmtKind};
use std::collections::HashMap;

const PRETYPED: &str = "__pre$";
const RECV: &str = "__recv$";
const VEC_ASSOCIATED: &[&str] = &["new", "with_capacity"];
const STRING_INTRINSICS: &[&str] = &[
    "is_empty",
    "clone",
    "as_str",
    "to_string",
    "to_owned",
    "as_bytes",
    "bytes",
    "push_str",
    "push",
];

enum Form {
    Value,
    Shared,
    Mutable,
}

impl TypeInference {
    pub(crate) fn stash_pretyped(&mut self, typed: TypedExpr) -> Expr {
        let name = format!("{PRETYPED}{}", self.recv_counter);
        self.recv_counter += 1;
        let span = typed.span;
        self.pretyped.insert(name.clone(), typed);
        Expr::new(ExprKind::Identifier(name), span)
    }

    pub(super) fn take_pretyped(&mut self, name: &str) -> Option<TypedExpr> {
        if !name.starts_with(PRETYPED) {
            return None;
        }
        self.pretyped.remove(name)
    }

    fn receiver_unknown(&mut self, member: &str, span: Span) {
        self.errors.push(TypeError::coded(
            "E0304",
            format!(
                "the type of the receiver of `{member}` is not known here; annotate the binding"
            ),
            "receiver of unknown type",
            span,
        ));
    }

    fn any_table_has(&self, member: &str) -> bool {
        self.methods
            .values()
            .chain(self.trait_methods.values())
            .any(|entries| entries.iter().any(|e| e.name == member))
            || Self::vec_receiver_method(member)
    }

    fn vec_receiver_method(member: &str) -> bool {
        super::primary::intrinsic_methods("Vec")
            .is_some_and(|names| names.contains(&member) && !VEC_ASSOCIATED.contains(&member))
    }

    fn string_method(
        &mut self,
        typed_object: TypedExpr,
        base: &InferType,
        via_ref: Option<bool>,
        member: &str,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let recv_span = typed_object.span;
        let recv = match via_ref {
            Some(_) => TypedExpr::new(
                TypedExprKind::Deref(Box::new(typed_object)),
                base.clone(),
                recv_span,
            ),
            None => typed_object,
        };
        let arity = usize::from(matches!(member, "push_str" | "push"));
        if args.len() != arity {
            self.errors.push(TypeError::coded(
                "E0302",
                format!(
                    "`{member}` takes {arity} argument{}, found {}",
                    if arity == 1 { "" } else { "s" },
                    args.len()
                ),
                "wrong number of arguments",
                span,
            ));
            return self.skip_args(args);
        }
        match member {
            "is_empty" => {
                let len = TypedExpr::new(
                    TypedExprKind::Member {
                        object: Box::new(recv),
                        member: "len".to_string(),
                    },
                    InferType::I64,
                    recv_span,
                );
                let zero = TypedExpr::new(TypedExprKind::Int(0), InferType::I64, span);
                (
                    TypedExprKind::Binary {
                        left: Box::new(len),
                        op: aelys_syntax::BinaryOp::Eq,
                        right: Box::new(zero),
                    },
                    InferType::Bool,
                )
            }
            "as_str" => (recv.kind, InferType::StrRef),
            "clone" => (recv.kind, base.clone()),
            "to_string" | "to_owned" => (recv.kind, InferType::String),
            "as_bytes" | "bytes" => (
                TypedExprKind::Member {
                    object: Box::new(recv),
                    member: "bytes".to_string(),
                },
                InferType::Slice {
                    elem: Box::new(InferType::U8),
                    mutable: false,
                },
            ),
            _ => {
                if *base == InferType::StrRef {
                    self.errors.push(TypeError::coded(
                        "E0320",
                        format!(
                            "`{member}` grows a `String`, and this receiver is a `&str`, a \
                             read-only string; make it owned with `String::from(..)` or \
                             `.to_string()`"
                        ),
                        "a `&str` cannot grow",
                        recv_span,
                    ));
                } else {
                    let writable = {
                        let env = &self.env;
                        crate::place_spine::place_is_writable(&recv, &|n: &str| env.is_mutable(n))
                    };
                    if !writable {
                        let message = match &recv.kind {
                            TypedExprKind::Identifier(name) => format!(
                                "cannot `{member}` onto the immutable binding `{name}`; declare \
                                 it `let mut {name}`"
                            ),
                            _ => format!("cannot `{member}` here: the string is not a mutable place"),
                        };
                        self.errors
                            .push(TypeError::coded("E0401", message, "not mutable", recv_span));
                    }
                }
                let arg = self.infer_expr(&args[0]);
                let want = if member == "push" {
                    InferType::Char
                } else {
                    InferType::StrRef
                };
                self.constraints.push(crate::constraint::Constraint::flows(
                    arg.ty.clone(),
                    want,
                    arg.span,
                    crate::constraint::ConstraintReason::Other(format!(
                        "the argument of `{member}`"
                    )),
                ));
                (
                    TypedExprKind::EnumVariant {
                        enum_name: "string".to_string(),
                        variant: member.to_string(),
                        tag: 0,
                        args: vec![recv, arg],
                    },
                    InferType::Null,
                )
            }
        }
    }

    pub(super) fn infer_method_call(
        &mut self,
        typed_object: TypedExpr,
        member: &str,
        args: &[Expr],
        span: Span,
    ) -> Result<(TypedExprKind, InferType), TypedExpr> {
        let seen = match &typed_object.ty {
            InferType::Var(_) => self.probe_type(&typed_object.ty),
            InferType::Ref { referent, .. } if matches!(referent.as_ref(), InferType::Var(_)) => {
                self.probe_type(&typed_object.ty)
            }
            other => other.clone(),
        };
        let (base, via_ref) = match &seen {
            InferType::Ref { referent, mutable } => ((**referent).clone(), Some(*mutable)),
            other => (other.clone(), None),
        };
        let dyn_trait = match &base {
            InferType::Dyn(q) => Some(q.clone()),
            other => crate::infer::objects::dyn_target(other).map(str::to_string),
        };
        if let Some(q) = dyn_trait {
            let mut recv = typed_object;
            recv.ty = seen.clone();
            if base.is_rc() && via_ref.is_some() {
                let rspan = recv.span;
                recv = TypedExpr::new(TypedExprKind::Deref(Box::new(recv)), base.clone(), rspan);
            }
            return Ok(self.dyn_method_call(recv, &q, member, args, span));
        }
        if let Some((tp, bounds)) = self.bounds_on_type_param(&base) {
            let Some(entry) = self.bound_method(&tp, &bounds, member, span) else {
                for a in args {
                    self.infer_expr(a);
                }
                return Ok((TypedExprKind::Null, InferType::Dynamic));
            };
            self.note_stub_use(&entry.fn_name);
            return Ok(self.desugar_method_call(
                typed_object,
                &base,
                via_ref,
                &tp,
                entry,
                args,
                span,
            ));
        }
        if base.is_str() && STRING_INTRINSICS.contains(&member) {
            return Ok(self.string_method(typed_object, &base, via_ref, member, args, span));
        }
        if let Some(key) = crate::infer::impls::builtin_type_key(&base)
            && let Some(entry) = self.builtin_entry(key, member)
        {
            return Ok(self.builtin_method_call(
                typed_object,
                &base,
                via_ref,
                key,
                entry,
                args,
                span,
            ));
        }
        if matches!(base, InferType::Vec(_)) && !Self::vec_receiver_method(member) {
            let key = crate::infer::impls::SLICE_KEY;
            if let Some(entry) = self.builtin_entry(key, member) {
                return Ok(self.builtin_method_call(
                    typed_object,
                    &base,
                    via_ref,
                    key,
                    entry,
                    args,
                    span,
                ));
            }
        }
        if let Some(key) = crate::infer::impls::builtin_type_key(&base)
            && let Some(module) = std_method_module(key, member).or_else(|| {
                matches!(base, InferType::Vec(_))
                    .then(|| std_method_module(crate::infer::impls::SLICE_KEY, member))
                    .flatten()
            })
        {
            self.errors.push(TypeError::coded(
                "E0304",
                format!(
                    "no method `{member}` on `{}`; it is in `std::{module}` (use std::{module};)",
                    method_shown(&base)
                ),
                "method of a module not imported",
                span,
            ));
            return Ok(self.skip_args(args));
        }
        if !matches!(
            base,
            InferType::Struct(..) | InferType::Enum(..) | InferType::Var(_) | InferType::Dynamic
        ) && !self.trait_candidates(&base, member).is_empty()
        {
            let key = base.to_string();
            return Ok(match self.lookup_for(&base, member, span) {
                MethodLookup::Found(entry) => {
                    self.note_stub_use(&entry.fn_name);
                    self.desugar_method_call(typed_object, &base, via_ref, &key, entry, args, span)
                }
                _ => {
                    for a in args {
                        self.infer_expr(a);
                    }
                    (TypedExprKind::Null, InferType::Dynamic)
                }
            });
        }
        match &base {
            InferType::Var(_) => {
                if let TypedExprKind::Call { callee, .. } = &typed_object.kind
                    && let TypedExprKind::Identifier(f) = &callee.kind
                    && let Some(rest) = f.strip_prefix(crate::infer::traits::STUB)
                {
                    let q = rest.split('$').next().unwrap_or(rest);
                    let m = rest.rsplit('$').next().unwrap_or(rest);
                    let t = crate::infer::traits::trait_shown(q);
                    self.errors.push(TypeError::coded(
                        "E0758",
                        format!(
                            "`{t}::{m}()` does not say which type implements `{t}`; call it on a \
                             type, as `M::{m}()`, or bind the result with a type annotation"
                        ),
                        "undecided Self",
                        typed_object.span,
                    ));
                    for a in args {
                        self.infer_expr(a);
                    }
                    return Ok((TypedExprKind::Null, InferType::Dynamic));
                }
                if member == "len" && args.is_empty() {
                    return Ok(self.member_of_typed(typed_object, member, span));
                }
                if self.any_table_has(member) {
                    self.receiver_unknown(member, span);
                    for a in args {
                        self.infer_expr(a);
                    }
                    return Ok((TypedExprKind::Null, InferType::Dynamic));
                }
                Err(typed_object)
            }
            InferType::Struct(name, targs) | InferType::Enum(name, targs) => {
                let is_struct = matches!(base, InferType::Struct(..));
                if is_struct
                    && self.type_table.field_type(name, targs, member).is_some()
                    && !self.has_method(&base, member)
                {
                    return Err(typed_object);
                }
                if is_struct && !self.type_table.has_struct(name) {
                    if self.any_table_has(member) {
                        return Ok(self.no_method_on(&typed_object.ty, member, args, span));
                    }
                    return Err(typed_object);
                }
                let name = name.clone();
                match self.lookup_for(&base, member, span) {
                    MethodLookup::Found(entry) => Ok(self.desugar_method_call(
                        typed_object,
                        &base,
                        via_ref,
                        &name,
                        entry,
                        args,
                        span,
                    )),
                    MethodLookup::Ambiguous => {
                        for a in args {
                            self.infer_expr(a);
                        }
                        Ok((TypedExprKind::Null, InferType::Dynamic))
                    }
                    MethodLookup::Missing => {
                        self.no_method(&base, member, span);
                        for a in args {
                            self.infer_expr(a);
                        }
                        Ok((TypedExprKind::Null, InferType::Dynamic))
                    }
                }
            }
            InferType::Vec(_) if Self::vec_receiver_method(member) => {
                let recv = match via_ref {
                    Some(_) => {
                        let span = typed_object.span;
                        let inner = TypedExpr::new(
                            TypedExprKind::Deref(Box::new(typed_object)),
                            base.clone(),
                            span,
                        );
                        self.stash_pretyped(inner)
                    }
                    None => self.stash_pretyped(typed_object),
                };
                let mut full = Vec::with_capacity(args.len() + 1);
                full.push(recv);
                full.extend(args.iter().cloned());
                Ok(self.infer_enum_variant("Vec", member, &full, span))
            }
            InferType::String if member == "as_bytes" && args.is_empty() => {
                let recv = match via_ref {
                    Some(_) => {
                        let span = typed_object.span;
                        TypedExpr::new(TypedExprKind::Deref(Box::new(typed_object)), base.clone(), span)
                    }
                    None => typed_object,
                };
                let object_span = recv.span;
                Ok((
                    TypedExprKind::Member {
                        object: Box::new(TypedExpr::new(recv.kind, recv.ty, object_span)),
                        member: "bytes".to_string(),
                    },
                    InferType::Slice {
                        elem: Box::new(InferType::U8),
                        mutable: false,
                    },
                ))
            }
            InferType::String
            | InferType::StrRef
            | InferType::Array(..)
            | InferType::Slice { .. }
            | InferType::Vec(_)
                if member == "len" =>
            {
                if !args.is_empty() {
                    self.errors.push(TypeError::coded(
                        "E0302",
                        format!(
                            "`len` takes no argument, found {}; expected 0, found {}",
                            args.len(),
                            args.len()
                        ),
                        "wrong number of arguments",
                        span,
                    ));
                }
                let recv = match via_ref {
                    Some(_) => {
                        let span = typed_object.span;
                        TypedExpr::new(
                            TypedExprKind::Deref(Box::new(typed_object)),
                            base.clone(),
                            span,
                        )
                    }
                    None => typed_object,
                };
                let object_span = recv.span;
                Ok((
                    TypedExprKind::Member {
                        object: Box::new(TypedExpr::new(recv.kind, recv.ty, object_span)),
                        member: "len".to_string(),
                    },
                    InferType::I64,
                ))
            }
            InferType::Rc(inner) => {
                if matches!(inner.as_ref(), InferType::Struct(n, a) if self.type_table.field_type(n, a, member).is_some())
                {
                    return Err(typed_object);
                }
                if matches!(inner.as_ref(), InferType::Struct(..) | InferType::Enum(..))
                    && matches!(self.lookup_quiet_for(inner, member), MethodLookup::Found(_))
                {
                    let span_obj = typed_object.span;
                    let handle = match via_ref {
                        Some(_) => TypedExpr::new(
                            TypedExprKind::Deref(Box::new(typed_object)),
                            base.clone(),
                            span_obj,
                        ),
                        None => typed_object,
                    };
                    let handle = self.stash_pretyped(handle);
                    let get = Expr::new(
                        ExprKind::EnumVariant {
                            enum_name: "Rc".to_string(),
                            variant: "get".to_string(),
                            args: vec![handle],
                            unqualified: false,
                        },
                        span_obj,
                    );
                    let callee = Expr::new(
                        ExprKind::Member {
                            object: Box::new(get),
                            member: member.to_string(),
                        },
                        span,
                    );
                    return Ok(self.infer_call_expr(&callee, args, span));
                }
                self.errors.push(TypeError::coded(
                    "E0304",
                    format!(
                        "no method `{member}` on `{}`; call it on `Rc::get(r)`",
                        method_shown(&base)
                    ),
                    "no auto-dereference through `Rc`",
                    span,
                ));
                for a in args {
                    self.infer_expr(a);
                }
                Ok((TypedExprKind::Null, InferType::Dynamic))
            }
            InferType::String
            | InferType::StrRef
            | InferType::Vec(_)
            | InferType::Slice { .. }
            | InferType::Array(..) => {
                self.no_builtin_method(&base, member, span);
                for a in args {
                    self.infer_expr(a);
                }
                Ok((TypedExprKind::Null, InferType::Dynamic))
            }
            InferType::Function { .. } | InferType::Tuple(_) | InferType::Dynamic => {
                Err(typed_object)
            }
            _ if self.any_table_has(member) => {
                Ok(self.no_method_on(&typed_object.ty, member, args, span))
            }
            _ => Err(typed_object),
        }
    }

    fn builtin_entry(&self, key: &str, member: &str) -> Option<MethodEntry> {
        self.builtin_methods
            .get(key)?
            .iter()
            .find(|e| e.name == member)
            .cloned()
    }

    fn skip_args(&mut self, args: &[Expr]) -> (TypedExprKind, InferType) {
        for a in args {
            self.infer_expr(a);
        }
        (TypedExprKind::Null, InferType::Dynamic)
    }

    #[allow(clippy::too_many_arguments)]
    fn builtin_method_call(
        &mut self,
        typed_object: TypedExpr,
        base: &InferType,
        via_ref: Option<bool>,
        key: &str,
        entry: MethodEntry,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        let member = entry.name.clone();
        let Some((params, tparams)) = self.method_sig(&entry) else {
            return self.skip_args(args);
        };
        if entry.receiver == Receiver::None || params.is_empty() {
            return self.no_method_on(&typed_object.ty, &member, args, span);
        }
        let head = match &params[0] {
            InferType::Ref { referent, .. } => (**referent).clone(),
            other => other.clone(),
        };
        let recv_elem = match (base, &head) {
            (InferType::Array(elem, _), _) | (InferType::Vec(elem), InferType::Slice { .. }) => {
                Some(InferType::Slice {
                    elem: elem.clone(),
                    mutable: false,
                })
            }
            _ => None,
        };
        let matched = match (&head, recv_elem.as_ref().unwrap_or(base)) {
            (InferType::Slice { elem: pe, .. }, InferType::Slice { elem: te, .. }) => {
                head_mismatch(pe, te, &tparams).is_none()
            }
            (h, b) => head_mismatch(h, b, &tparams).is_none(),
        };
        if !matched {
            self.errors.push(TypeError::coded(
                "E0304",
                format!(
                    "no method `{member}` on `{}`; `{member}` is defined for `{}`",
                    method_shown(base),
                    builtin_head_shown(&head)
                ),
                "receiver does not match the impl",
                span,
            ));
            return self.skip_args(args);
        }
        let InferType::Slice {
            mutable: wants_mut, ..
        } = head
        else {
            return self.desugar_method_call(typed_object, base, via_ref, key, entry, args, span);
        };
        self.note_stub_use(&entry.fn_name);
        if params.len() != args.len() + 1 {
            self.arity_error(&member, params.len().saturating_sub(1), args.len(), span);
            return self.skip_args(args);
        }
        let recv_span = typed_object.span;
        let callee = Expr::new(ExprKind::Identifier(entry.fn_name.clone()), span);
        let object = match via_ref {
            Some(_) => TypedExpr::new(
                TypedExprKind::Deref(Box::new(typed_object)),
                base.clone(),
                recv_span,
            ),
            None => typed_object,
        };
        let recv = match base {
            InferType::Slice { mutable, .. } => {
                if wants_mut && !*mutable {
                    self.errors.push(TypeError::coded(
                        "E0422",
                        format!(
                            "`{member}` takes `&mut self`, and its receiver is a shared slice `{}`; \
                             mutation through a shared reference",
                            method_shown(base)
                        ),
                        "shared slice",
                        recv_span,
                    ));
                    return self.skip_args(args);
                }
                object
            }
            InferType::Array(elem, _) | InferType::Vec(elem) => {
                let writable = {
                    let env = &self.env;
                    crate::place_spine::place_is_writable(&object, &|n: &str| env.is_mutable(n))
                };
                if wants_mut && !writable {
                    let before = self.errors.len();
                    self.require_mutable_root(&object, &member, recv_span);
                    if self.errors.len() == before {
                        self.errors.push(TypeError::coded(
                            "E0417",
                            format!(
                                "`{member}` takes `&mut self`, and its receiver is not a mutable \
                                 place"
                            ),
                            "immutable receiver",
                            recv_span,
                        ));
                    }
                    return self.skip_args(args);
                }
                let range = TypedExpr::new(
                    TypedExprKind::Range {
                        start: None,
                        end: None,
                        inclusive: false,
                    },
                    InferType::Range,
                    recv_span,
                );
                TypedExpr::new(
                    TypedExprKind::Slice {
                        object: Box::new(object),
                        range: Box::new(range),
                    },
                    InferType::Slice {
                        elem: elem.clone(),
                        // a mutable view of a vec detaches its buffer, so only a writer asks for one
                        mutable: writable && (wants_mut || matches!(base, InferType::Array(..))),
                    },
                    recv_span,
                )
            }
            _ => return self.no_method_on(base, &member, args, span),
        };
        let recv = self.stash_pretyped(recv);
        self.finish_method_call(callee, recv, args, span, None)
    }

    fn no_method_on(
        &mut self,
        ty: &InferType,
        member: &str,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        self.errors.push(TypeError::coded(
            "E0304",
            format!("no method `{member}` on `{}`", method_shown(ty)),
            "unknown method",
            span,
        ));
        for a in args {
            self.infer_expr(a);
        }
        (TypedExprKind::Null, InferType::Dynamic)
    }

    fn no_method(&mut self, base: &InferType, member: &str, span: Span) {
        let key = match base {
            InferType::Struct(n, _) | InferType::Enum(n, _) => n.clone(),
            _ => String::new(),
        };
        let shown = source_type_name(&key).to_string();
        if let MethodLookup::Found(entry) = self.lookup_quiet(&key, member)
            && entry.receiver == Receiver::None
        {
            self.errors.push(TypeError::coded(
                "E0304",
                format!("`{member}` has no `self` parameter; call it as `{shown}::{member}(..)`"),
                "associated function called as a method",
                span,
            ));
            return;
        }
        let names = self.method_names(&key);
        let listing = if names.is_empty() {
            format!("`{shown}` has no method")
        } else {
            format!(
                "methods of `{shown}`: {}",
                names
                    .iter()
                    .map(|n| format!("`{n}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            )
        };
        self.errors.push(TypeError::coded(
            "E0304",
            format!("no method `{member}` on `{shown}`; {listing}"),
            "unknown method",
            span,
        ));
    }

    fn no_builtin_method(&mut self, base: &InferType, member: &str, span: Span) {
        let shown = method_shown(base);
        let known: Vec<&str> = match base {
            InferType::Vec(_) => super::primary::intrinsic_methods("Vec")
                .unwrap_or(&[])
                .iter()
                .copied()
                .filter(|n| !VEC_ASSOCIATED.contains(n))
                .collect(),
            InferType::String | InferType::StrRef => {
                let grows = |n: &&str| *base == InferType::String || !n.starts_with("push");
                std::iter::once("len")
                    .chain(STRING_INTRINSICS.iter().copied().filter(grows))
                    .collect()
            }
            _ => vec!["len"],
        };
        let std_hint = match std_function_for(base, member) {
            Some(module) => format!(
                "; the std function is `{module}::{member}`, called as `{module}::{member}(x, ..)` \
                 (use std::{module};)"
            ),
            None => String::new(),
        };
        self.errors.push(TypeError::coded(
            "E0304",
            format!(
                "no method `{member}` on `{shown}`; its methods are {}{std_hint}",
                known
                    .iter()
                    .map(|n| format!("`{n}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            "unknown method",
            span,
        ));
    }

    fn lookup_quiet_for(&self, ty: &InferType, member: &str) -> MethodLookup {
        let key = match ty {
            InferType::Struct(n, _) | InferType::Enum(n, _) => n.clone(),
            _ => return MethodLookup::Missing,
        };
        match self.lookup_quiet(&key, member) {
            MethodLookup::Missing if !self.trait_candidates(ty, member).is_empty() => {
                MethodLookup::Found(self.trait_candidates(ty, member).remove(0))
            }
            other => other,
        }
    }

    fn lookup_quiet(&self, key: &str, member: &str) -> MethodLookup {
        let hits: Vec<&MethodEntry> = [&self.methods, &self.trait_methods]
            .into_iter()
            .filter_map(|t| t.get(key))
            .flat_map(|entries| entries.iter().filter(|e| e.name == member))
            .collect();
        match hits.as_slice() {
            [one] => MethodLookup::Found((*one).clone()),
            [] => MethodLookup::Missing,
            _ => MethodLookup::Ambiguous,
        }
    }

    fn check_method_visibility(&mut self, key: &str, entry: &MethodEntry, span: Span) {
        if entry.is_pub || entry.module.is_empty() {
            return;
        }
        let path = self.module_path();
        let ty = if crate::modules::module_of_type(key).is_some() {
            crate::modules::strip_type_head(key).to_string()
        } else if crate::infer::is_builtin_key(key) {
            format!("{}.{}", entry.module, source_type_name(key))
        } else if path.is_empty() {
            key.to_string()
        } else {
            format!("{path}.{key}")
        };
        self.errors.push(TypeError::coded(
            "E0610",
            format!("method `{}` of `{ty}` is not public", entry.name),
            "method is not public",
            span,
        ));
    }

    fn method_sig(&self, entry: &MethodEntry) -> Option<(Vec<InferType>, Vec<String>)> {
        let ty = self
            .env
            .lookup_function_ref(&entry.fn_name)
            .or_else(|| self.env.lookup(&entry.fn_name))?;
        let InferType::Function { params, .. } = ty else {
            return None;
        };
        let hidden = self.hidden_count(&entry.fn_name);
        let params: Vec<InferType> = params[..params.len().saturating_sub(hidden)].to_vec();
        let tparams = match &entry.value {
            Some(v) => v.type_params.clone(),
            None => self
                .method_tparams
                .get(&entry.fn_name)
                .cloned()
                .unwrap_or_default(),
        };
        Some((params.clone(), tparams))
    }

    fn arity_error(&mut self, member: &str, expected: usize, found: usize, span: Span) {
        self.errors.push(TypeError::coded(
            "E0302",
            format!(
                "`{member}` takes {expected} argument{} but {found} {} supplied; expected {expected}, found {found}",
                if expected == 1 { "" } else { "s" },
                if found == 1 { "was" } else { "were" },
            ),
            "wrong number of arguments",
            span,
        ));
    }

    #[allow(clippy::too_many_arguments)]
    fn desugar_method_call(
        &mut self,
        typed_object: TypedExpr,
        base: &InferType,
        via_ref: Option<bool>,
        key: &str,
        entry: MethodEntry,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        self.check_method_visibility(key, &entry, span);
        self.note_stub_use(&entry.fn_name);
        let member = entry.name.clone();
        let Some((params, tparams)) = self.method_sig(&entry) else {
            for a in args {
                self.infer_expr(a);
            }
            return (TypedExprKind::Null, InferType::Dynamic);
        };
        if entry.receiver == Receiver::None {
            self.no_method(base, &member, span);
            for a in args {
                self.infer_expr(a);
            }
            return (TypedExprKind::Null, InferType::Dynamic);
        }
        if params.len() != args.len() + 1 {
            self.arity_error(&member, params.len().saturating_sub(1), args.len(), span);
            for a in args {
                self.infer_expr(a);
            }
            return (TypedExprKind::Null, InferType::Dynamic);
        }
        let head = match &params[0] {
            InferType::Ref { referent, .. } => (**referent).clone(),
            other => other.clone(),
        };
        if let Some((pattern, found)) = head_mismatch(&head, base, &tparams) {
            let _ = pattern;
            self.errors.push(TypeError::coded(
                "E0301",
                format!(
                    "`{member}` is defined for `{}`, and the receiver is `{}`",
                    shown_with_params(&head),
                    method_shown(&found)
                ),
                "receiver does not match the impl",
                typed_object.span,
            ));
            for a in args {
                self.infer_expr(a);
            }
            return (TypedExprKind::Null, InferType::Dynamic);
        }

        let form = match entry.receiver {
            Receiver::Value => Form::Value,
            Receiver::Ref => Form::Shared,
            _ => Form::Mutable,
        };
        let recv_span = typed_object.span;
        let is_place = crate::place_spine::denotes_a_place(&typed_object);

        if let (Form::Mutable, Some(false)) = (&form, via_ref) {
            self.errors.push(TypeError::coded(
                "E0422",
                format!(
                    "`{member}` takes `&mut self`, and its receiver is a shared reference `{}`; \
                     mutation through a shared reference",
                    method_shown(&typed_object.ty)
                ),
                "shared reference",
                recv_span,
            ));
            for a in args {
                self.infer_expr(a);
            }
            return (TypedExprKind::Null, InferType::Dynamic);
        }

        let callee = Expr::new(ExprKind::Identifier(entry.fn_name.clone()), span);
        let deref = |inner: TypedExpr| {
            let span = inner.span;
            TypedExpr::new(TypedExprKind::Deref(Box::new(inner)), base.clone(), span)
        };

        match (form, via_ref) {
            (Form::Value, None) => {
                let recv = self.stash_pretyped(typed_object);
                self.finish_method_call(callee, recv, args, span, None)
            }
            (Form::Value, Some(_)) => {
                let recv = self.stash_pretyped(deref(typed_object));
                self.finish_method_call(callee, recv, args, span, None)
            }
            (form, Some(_)) => {
                let mutable = matches!(form, Form::Mutable);
                let target = self.stash_pretyped(deref(typed_object));
                let recv = Expr::new(
                    ExprKind::Reference {
                        mutable,
                        operand: Box::new(target),
                    },
                    recv_span,
                );
                self.finish_method_call(callee, recv, args, span, Some(member))
            }
            (form, None) if is_place => {
                let mutable = matches!(form, Form::Mutable);
                if mutable {
                    self.require_mutable_root(&typed_object, &member, recv_span);
                }
                let target = self.stash_pretyped(typed_object);
                let recv = Expr::new(
                    ExprKind::Reference {
                        mutable,
                        operand: Box::new(target),
                    },
                    recv_span,
                );
                self.finish_method_call(callee, recv, args, span, Some(member))
            }
            (form, None) => {
                let mutable = matches!(form, Form::Mutable);
                if let Some(InferType::Function { ret, .. }) =
                    self.env.lookup_function_ref(&entry.fn_name).cloned()
                    && contains_ref(&ret)
                {
                    self.errors.push(TypeError::coded(
                        "E0421",
                        format!(
                            "the receiver of `{member}` is a temporary and `{member}` returns a \
                             reference; bind it to a name first"
                        ),
                        "temporary receiver",
                        recv_span,
                    ));
                }
                let (hoisted, typed_object) = split_receiver_block(typed_object);
                let name = format!("{RECV}{}", self.recv_counter);
                self.recv_counter += 1;
                let init = self.stash_pretyped(typed_object);
                let binding = Expr::new(ExprKind::Identifier(name.clone()), recv_span);
                let recv = Expr::new(
                    ExprKind::Reference {
                        mutable,
                        operand: Box::new(binding),
                    },
                    recv_span,
                );
                let mut full = Vec::with_capacity(args.len() + 1);
                full.push(recv);
                full.extend(args.iter().cloned());
                let call = Expr::new(
                    ExprKind::Call {
                        callee: Box::new(callee),
                        args: full,
                    },
                    span,
                );
                let block = Expr::new(
                    ExprKind::Block {
                        stmts: vec![Stmt::new(
                            StmtKind::Let {
                                name,
                                mutable,
                                type_annotation: None,
                                initializer: init,
                                is_pub: false,
                            },
                            recv_span,
                        )],
                        tail: Box::new(call),
                    },
                    span,
                );
                let typed = self.infer_expr(&block);
                let mut typed = typed;
                if let TypedExprKind::Block { tail, stmts } = &mut typed.kind {
                    mark_receiver(tail, &member);
                    stmts.splice(0..0, hoisted);
                }
                (typed.kind, typed.ty)
            }
        }
    }

    fn finish_method_call(
        &mut self,
        callee: Expr,
        recv: Expr,
        args: &[Expr],
        span: Span,
        receiver_of: Option<String>,
    ) -> (TypedExprKind, InferType) {
        let mut full = Vec::with_capacity(args.len() + 1);
        full.push(recv);
        full.extend(args.iter().cloned());
        let (mut kind, ty) = self.infer_call_expr(&callee, &full, span);
        if let Some(member) = receiver_of
            && let TypedExprKind::Call { args, .. } = &mut kind
            && let Some(first) = args.first_mut()
            && let TypedExprKind::Reference { receiver_of, .. } = &mut first.kind
        {
            *receiver_of = Some(member);
        }
        (kind, ty)
    }

    fn require_mutable_root(&mut self, place: &TypedExpr, member: &str, span: Span) {
        let mut cur = place;
        loop {
            match &cur.kind {
                TypedExprKind::Grouping(inner) => cur = inner,
                TypedExprKind::Member { object, .. } | TypedExprKind::Index { object, .. } => {
                    if matches!(
                        object.ty,
                        InferType::Ref { .. } | InferType::Rc(_) | InferType::Slice { .. }
                    ) {
                        return;
                    }
                    cur = object;
                }
                TypedExprKind::Identifier(name) => {
                    if self.env.lookup_local(name).is_some() && !self.env.is_mutable(name) {
                        let fix = if name == "self" {
                            "rebind it with `let mut s = self`".to_string()
                        } else {
                            format!("declare it with `let mut {name}`")
                        };
                        self.errors.push(TypeError::coded(
                            "E0417",
                            format!(
                                "`{member}` takes `&mut self`, and `{name}` is not mutable; {fix}"
                            ),
                            "immutable receiver",
                            span,
                        ));
                    }
                    return;
                }
                _ => return,
            }
        }
    }

    pub(super) fn infer_method_path_call(
        &mut self,
        key: &str,
        entry: MethodEntry,
        args: &[Expr],
        span: Span,
    ) -> (TypedExprKind, InferType) {
        self.check_method_visibility(key, &entry, span);
        self.note_stub_use(&entry.fn_name);
        if let Some((params, _)) = self.method_sig(&entry)
            && params.len() != args.len()
        {
            if args.is_empty() {
                let path = format!("{}::{}", source_type_name(key), entry.name);
                self.errors.push(TypeError::coded(
                    "E0302",
                    format!(
                        "`{path}` takes {} argument{} and none was supplied; a method is not a \
                         value, call it as `{path}(..)`",
                        params.len(),
                        if params.len() == 1 { "" } else { "s" }
                    ),
                    "method path without arguments",
                    span,
                ));
                return (TypedExprKind::Null, InferType::Dynamic);
            }
            self.arity_error(&entry.name, params.len(), args.len(), span);
            for a in args {
                self.infer_expr(a);
            }
            return (TypedExprKind::Null, InferType::Dynamic);
        }
        let callee = Expr::new(ExprKind::Identifier(entry.fn_name.clone()), span);
        let (kind, ty) = self.infer_call_expr(&callee, args, span);
        if entry.receiver == Receiver::None
            && entry.fn_name.starts_with(crate::infer::traits::STUB)
            && let Some(self_ty) = self.nominal_without_args(key)
        {
            self.pin_self_result(&entry.fn_name, &ty, self_ty, span);
        }
        (kind, ty)
    }

    fn nominal_without_args(&self, key: &str) -> Option<InferType> {
        if let Some(d) = self.type_table.get_struct(key) {
            return d
                .type_params
                .is_empty()
                .then(|| InferType::Struct(key.to_string(), Vec::new()));
        }
        let d = self.type_table.get_enum(key)?;
        d.type_params
            .is_empty()
            .then(|| InferType::Enum(key.to_string(), Vec::new()))
    }
}

fn split_receiver_block(recv: TypedExpr) -> (Vec<crate::typed_ast::TypedStmt>, TypedExpr) {
    let hoistable = matches!(&recv.kind, TypedExprKind::Block { stmts, .. }
        if !stmts.is_empty() && stmts.iter().all(|s| matches!(&s.kind,
            crate::typed_ast::TypedStmtKind::Let { name, .. } if name.starts_with(RECV))));
    if !hoistable {
        return (Vec::new(), recv);
    }
    match recv.kind {
        TypedExprKind::Block { stmts, tail } => (stmts, *tail),
        _ => unreachable!("checked above"),
    }
}

fn mark_receiver(call: &mut TypedExpr, member: &str) {
    if let TypedExprKind::Call { args, .. } = &mut call.kind
        && let Some(first) = args.first_mut()
        && let TypedExprKind::Reference { receiver_of, .. } = &mut first.kind
    {
        *receiver_of = Some(member.to_string());
    }
}

fn contains_ref(ty: &InferType) -> bool {
    match ty {
        InferType::Ref { .. } | InferType::Slice { .. } => true,
        InferType::Struct(_, args) | InferType::Enum(_, args) | InferType::Tuple(args) => {
            args.iter().any(contains_ref)
        }
        InferType::Array(inner, _) | InferType::Vec(inner) | InferType::Rc(inner) => {
            contains_ref(inner)
        }
        _ => false,
    }
}

fn is_param(ty: &InferType, tparams: &[String]) -> Option<String> {
    match ty {
        InferType::Struct(name, args) if args.is_empty() && tparams.contains(name) => {
            Some(name.clone())
        }
        _ => None,
    }
}

fn is_concrete(ty: &InferType) -> bool {
    match ty {
        InferType::Var(_) | InferType::Dynamic => false,
        InferType::Struct(_, a) | InferType::Enum(_, a) | InferType::Tuple(a) => {
            a.iter().all(is_concrete)
        }
        InferType::Array(i, _) | InferType::Vec(i) | InferType::Rc(i) => is_concrete(i),
        InferType::Ref { referent, .. } => is_concrete(referent),
        InferType::Slice { elem, .. } => is_concrete(elem),
        InferType::Function { params, ret, .. } => {
            params.iter().all(is_concrete) && is_concrete(ret)
        }
        _ => true,
    }
}

fn head_mismatch(
    head: &InferType,
    recv: &InferType,
    tparams: &[String],
) -> Option<(InferType, InferType)> {
    let mut bound: HashMap<String, InferType> = HashMap::new();
    if matches_head(head, recv, tparams, &mut bound) {
        None
    } else {
        Some((head.clone(), recv.clone()))
    }
}

fn matches_head(
    pat: &InferType,
    ty: &InferType,
    tparams: &[String],
    bound: &mut HashMap<String, InferType>,
) -> bool {
    if let Some(name) = is_param(pat, tparams) {
        if !is_concrete(ty) {
            return true;
        }
        if let Some(prev) = bound.get(&name) {
            return prev.equiv(ty);
        }
        bound.insert(name, ty.clone());
        return true;
    }
    if !is_concrete(ty) && matches!(ty, InferType::Var(_) | InferType::Dynamic) {
        return true;
    }
    match (pat, ty) {
        (InferType::Vec(p), InferType::Vec(t))
        | (InferType::Slice { elem: p, .. }, InferType::Slice { elem: t, .. })
        | (InferType::Array(p, _), InferType::Array(t, _))
        | (InferType::Rc(p), InferType::Rc(t)) => matches_head(p, t, tparams, bound),
        (InferType::Struct(a, pa), InferType::Struct(b, ta))
        | (InferType::Enum(a, pa), InferType::Enum(b, ta)) => {
            a == b
                && pa.len() == ta.len()
                && pa
                    .iter()
                    .zip(ta.iter())
                    .all(|(p, t)| matches_head(p, t, tparams, bound))
        }
        _ => !is_concrete(pat) || !is_concrete(ty) || pat.equiv(ty),
    }
}

fn shown_with_params(ty: &InferType) -> String {
    ty.to_string()
}

const STD_STR: &str = include_str!("../../../../std/str.aelys");
const STD_VEC: &str = include_str!("../../../../std/vec.aelys");
const STD_SLICE: &str = include_str!("../../../../std/slice.aelys");
const STD_MATH: &str = include_str!("../../../../std/math.aelys");

fn std_impl_key(line: &str) -> Option<&'static str> {
    let rest = line.strip_prefix("impl")?;
    let rest = rest.trim_start();
    let rest = match rest.strip_prefix('<') {
        Some(r) => r.split_once('>')?.1.trim_start(),
        None => rest,
    };
    if rest.starts_with('[') {
        return Some(crate::infer::impls::SLICE_KEY);
    }
    let head: String = rest
        .chars()
        .take_while(|c| c.is_alphanumeric() || *c == '_')
        .collect();
    crate::infer::builtin_key_of_prefix(&head).filter(|k| *k != crate::infer::impls::SLICE_KEY)
}

fn std_method_module(key: &str, member: &str) -> Option<&'static str> {
    for (module, src) in [
        ("str", STD_STR),
        ("vec", STD_VEC),
        ("slice", STD_SLICE),
        ("math", STD_MATH),
    ] {
        let mut current: Option<&str> = None;
        for line in src.lines() {
            if line.starts_with("impl") {
                current = std_impl_key(line);
                continue;
            }
            if line.starts_with('}') {
                current = None;
                continue;
            }
            if current != Some(key) {
                continue;
            }
            let t = line.trim_start();
            let t = t.strip_prefix("pub ").unwrap_or(t);
            let t = t.strip_prefix("nogc ").unwrap_or(t);
            let Some(t) = t.strip_prefix("fn ") else {
                continue;
            };
            let name: String = t
                .chars()
                .take_while(|c| c.is_alphanumeric() || *c == '_')
                .collect();
            if name == member {
                return Some(module);
            }
        }
    }
    None
}

fn std_function_for(base: &InferType, member: &str) -> Option<&'static str> {
    let candidates: &[(&'static str, &str, &[&str])] = match base {
        InferType::String | InferType::StrRef => &[("str", STD_STR, &["string", "str"])],
        InferType::Vec(_) => &[("vec", STD_VEC, &["Vec<"]), ("slice", STD_SLICE, &["["])],
        InferType::Array(..) | InferType::Slice { .. } => &[("slice", STD_SLICE, &["["])],
        _ => &[],
    };
    let head = format!("pub fn {member}(");
    candidates.iter().find_map(|(module, src, accepts)| {
        src.lines().find_map(|line| {
            let rest = line.trim_start().strip_prefix(head.as_str())?;
            let first = rest.split([',', ')']).next()?;
            let ty = first.split_once(':')?.1.trim();
            accepts.iter().any(|a| ty.contains(a)).then_some(*module)
        })
    })
}

fn builtin_head_shown(head: &InferType) -> String {
    match head {
        InferType::Slice { elem, .. } => format!("[{elem}]"),
        other => other.to_string(),
    }
}

pub(crate) fn method_shown(ty: &InferType) -> String {
    ty.to_string()
}

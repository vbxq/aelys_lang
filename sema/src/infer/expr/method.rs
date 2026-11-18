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

enum Form {
    Value,
    Shared,
    Mutable,
}

impl TypeInference {
    pub(super) fn stash_pretyped(&mut self, typed: TypedExpr) -> Expr {
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
        match &base {
            InferType::Var(_) => {
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
                    && self
                        .type_table
                        .field_type(name, targs, member)
                        .is_some()
                {
                    return Err(typed_object);
                }
                if is_struct && !self.type_table.has_struct(name) {
                    return Err(typed_object);
                }
                let name = name.clone();
                match self.lookup_method(&name, member, span) {
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
            InferType::String | InferType::Array(..) | InferType::Slice { .. } | InferType::Vec(_)
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
            InferType::String | InferType::Vec(_) | InferType::Slice { .. } | InferType::Array(..) => {
                self.no_builtin_method(&base, member, span);
                for a in args {
                    self.infer_expr(a);
                }
                Ok((TypedExprKind::Null, InferType::Dynamic))
            }
            _ => Err(typed_object),
        }
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
                format!(
                    "`{member}` has no `self` parameter; call it as `{shown}::{member}(..)`"
                ),
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
            _ => vec!["len"],
        };
        let std_hint = match std_function_for(base, member) {
            Some(module) => format!(
                "; the std function is `{module}.{member}`, called as `{module}.{member}(x, ..)` \
                 (needs std.{module})"
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
        self.errors.push(TypeError::coded(
            "E0610",
            format!(
                "method `{}` of `{}.{}` is not public",
                entry.name,
                entry.module,
                source_type_name(key)
            ),
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
                if let Some(InferType::Function { ret, .. }) = self
                    .env
                    .lookup_function_ref(&entry.fn_name)
                    .cloned()
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
                if let TypedExprKind::Block { tail, .. } = &mut typed.kind {
                    mark_receiver(tail, &member);
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
        if let Some((params, _)) = self.method_sig(&entry)
            && params.len() != args.len()
        {
            self.arity_error(&entry.name, params.len(), args.len(), span);
            for a in args {
                self.infer_expr(a);
            }
            return (TypedExprKind::Null, InferType::Dynamic);
        }
        let callee = Expr::new(ExprKind::Identifier(entry.fn_name.clone()), span);
        self.infer_call_expr(&callee, args, span)
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
            return prev == ty;
        }
        bound.insert(name, ty.clone());
        return true;
    }
    if !is_concrete(ty) && matches!(ty, InferType::Var(_) | InferType::Dynamic) {
        return true;
    }
    match (pat, ty) {
        (InferType::Struct(a, pa), InferType::Struct(b, ta))
        | (InferType::Enum(a, pa), InferType::Enum(b, ta)) => {
            a == b
                && pa.len() == ta.len()
                && pa
                    .iter()
                    .zip(ta.iter())
                    .all(|(p, t)| matches_head(p, t, tparams, bound))
        }
        _ => !is_concrete(pat) || !is_concrete(ty) || pat == ty,
    }
}

fn shown_with_params(ty: &InferType) -> String {
    ty.to_string()
}

const STD_STR: &str = include_str!("../../../../std/str.aelys");
const STD_VEC: &str = include_str!("../../../../std/vec.aelys");
const STD_SLICE: &str = include_str!("../../../../std/slice.aelys");

fn std_function_for(base: &InferType, member: &str) -> Option<&'static str> {
    let candidates: &[(&'static str, &str, &[&str])] = match base {
        InferType::String => &[("str", STD_STR, &["string"])],
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

pub(crate) fn method_shown(ty: &InferType) -> String {
    ty.to_string()
}

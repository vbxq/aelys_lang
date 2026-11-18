use super::TypeInference;
use crate::constraint::TypeError;
use crate::modules::{
    ImplDef, ItemKind, MethodEntry, ModuleValue, TraitDef, TraitMethodSig, qualify_infer_type,
    qualify_type_name, source_type_name,
};
use crate::types::InferType;
use aelys_syntax::{
    BinaryOp, Expr, ExprKind, FmtStringPart, Function, ImplDecl, Parameter, Receiver, RefKind,
    Span, Stmt, StmtKind, TraitDecl, TraitRef, TypeAnnotation, TypeBounds, UnaryOp,
};
use std::collections::{HashMap, HashSet};

pub(crate) const STUB: &str = "__trait$";
pub(crate) const IMPL: &str = "__impl$";
const SELF: &str = "Self";

pub(crate) fn stub_name(q: &str, m: &str) -> String {
    format!("{STUB}{q}${m}")
}

pub(crate) fn trait_shown(q: &str) -> &str {
    source_type_name(q)
}

pub(crate) fn provider_shown(fn_name: &str) -> String {
    for prefix in [IMPL, STUB] {
        if let Some(rest) = fn_name.strip_prefix(prefix) {
            let q = rest.split('$').next().unwrap_or(rest);
            let m = rest.rsplit('$').next().unwrap_or(rest);
            let q = q.strip_prefix("__q.").unwrap_or(q);
            return format!("{q}::{m}");
        }
    }
    match fn_name.split_once("::") {
        Some((t, m)) => format!("{}::{m}", source_type_name(t)),
        None => fn_name.to_string(),
    }
}

fn self_ann(span: Span, reference: Option<RefKind>) -> TypeAnnotation {
    let mut ann = TypeAnnotation::new(SELF.to_string(), span);
    ann.reference = reference;
    ann
}

fn recv_param(
    receiver: Receiver,
    mut_self: bool,
    head: &TypeAnnotation,
    span: Span,
) -> Option<Parameter> {
    let reference = match receiver {
        Receiver::None => return None,
        Receiver::Value => None,
        Receiver::Ref => Some(RefKind::Shared),
        Receiver::RefMut => Some(RefKind::Mut),
    };
    let mut ann = head.clone();
    ann.span = span;
    ann.reference = reference;
    Some(Parameter::new(
        "self".to_string(),
        mut_self && receiver == Receiver::Value,
        Some(ann),
        span,
    ))
}

fn self_ref() -> InferType {
    InferType::Ref {
        referent: Box::new(InferType::Struct(SELF.to_string(), Vec::new())),
        mutable: false,
    }
}

fn mentions_self(ann: &TypeAnnotation) -> bool {
    ann.name == SELF
        || ann.type_param.as_deref().is_some_and(mentions_self)
        || ann.type_params.iter().any(mentions_self)
        || ann.fn_params.iter().flatten().any(mentions_self)
        || ann.fn_ret.as_deref().is_some_and(mentions_self)
        || ann.tuple_elems.iter().flatten().any(mentions_self)
}

fn ident(name: &str, span: Span) -> Expr {
    Expr::new(ExprKind::Identifier(name.to_string()), span)
}

fn deref(name: &str, span: Span) -> Expr {
    Expr::new(ExprKind::Deref(Box::new(ident(name, span))), span)
}

fn ret(e: Expr, span: Span) -> Stmt {
    Stmt::new(StmtKind::Return(Some(e)), span)
}

pub(crate) fn builtin_traits() -> Vec<TraitDef> {
    let f = |params: Vec<InferType>, ret: InferType| InferType::Function {
        params,
        ret: Box::new(ret),
        nogc: false,
    };
    let sig = |q: &str, name: &str, ty: InferType| TraitMethodSig {
        name: name.to_string(),
        receiver: Receiver::Ref,
        has_default: false,
        stub: stub_name(q, name),
        ty,
    };
    vec![
        TraitDef {
            name: "eq".to_string(),
            qualified: "eq".to_string(),
            module: String::new(),
            is_pub: true,
            methods: vec![sig("eq", "eq", f(vec![self_ref(), self_ref()], InferType::Bool))],
        },
        TraitDef {
            name: "ord".to_string(),
            qualified: "ord".to_string(),
            module: String::new(),
            is_pub: true,
            methods: vec![sig("ord", "cmp", f(vec![self_ref(), self_ref()], InferType::I64))],
        },
        TraitDef {
            name: "Display".to_string(),
            qualified: "Display".to_string(),
            module: String::new(),
            is_pub: true,
            methods: vec![sig(
                "Display",
                "to_string",
                f(vec![self_ref()], InferType::String),
            )],
        },
    ]
}

fn builtin_stub(q: &str, span: Span) -> Function {
    let recv = Parameter::new(
        "self".to_string(),
        false,
        Some(self_ann(span, Some(RefKind::Shared))),
        span,
    );
    let other = Parameter::new(
        "o".to_string(),
        false,
        Some(self_ann(span, Some(RefKind::Shared))),
        span,
    );
    let cmp = |op: BinaryOp| {
        Expr::new(
            ExprKind::Binary {
                left: Box::new(deref("self", span)),
                op,
                right: Box::new(deref("o", span)),
            },
            span,
        )
    };
    let mut bounds = TypeBounds::default();
    let (name, params, ret_ty, body) = match q {
        "eq" => {
            bounds.eq = true;
            (
                "eq",
                vec![recv, other],
                "bool",
                vec![ret(cmp(BinaryOp::Eq), span)],
            )
        }
        "ord" => {
            bounds.ord = true;
            let branch = |op: BinaryOp, v: i64| {
                Stmt::new(
                    StmtKind::If {
                        condition: cmp(op),
                        then_branch: Box::new(Stmt::new(
                            StmtKind::Block(vec![ret(Expr::new(ExprKind::Int(v), span), span)]),
                            span,
                        )),
                        else_branch: None,
                    },
                    span,
                )
            };
            (
                "cmp",
                vec![recv, other],
                "i64",
                vec![
                    branch(BinaryOp::Lt, -1),
                    branch(BinaryOp::Gt, 1),
                    ret(Expr::new(ExprKind::Int(0), span), span),
                ],
            )
        }
        _ => {
            bounds.traits.push(TraitRef {
                name: "Display".to_string(),
                span,
            });
            (
                "to_string",
                vec![recv],
                "string",
                vec![ret(
                    Expr::new(
                        ExprKind::FmtString(vec![FmtStringPart::Expr(Box::new(deref(
                            "self", span,
                        )))]),
                        span,
                    ),
                    span,
                )],
            )
        }
    };
    Function {
        name: stub_name(q, name),
        type_params: vec![SELF.to_string()],
        bounds: vec![bounds],
        params,
        return_type: Some(TypeAnnotation::new(ret_ty.to_string(), span)),
        body,
        decorators: Vec::new(),
        is_pub: true,
        is_nogc: false,
        foreign: None,
        span,
    }
}

pub(crate) fn type_key(ty: &InferType, params: &[String]) -> String {
    match ty {
        InferType::Struct(n, a) | InferType::Enum(n, a) => {
            if a.is_empty()
                && let Some(i) = params.iter().position(|p| p == n)
            {
                return format!("${i}");
            }
            if a.is_empty() {
                n.clone()
            } else {
                let args: Vec<String> = a.iter().map(|t| type_key(t, params)).collect();
                format!("{n}[{}]", args.join(","))
            }
        }
        InferType::Tuple(es) => {
            let args: Vec<String> = es.iter().map(|t| type_key(t, params)).collect();
            format!("({})", args.join(","))
        }
        InferType::Array(i, n) => format!("[{};{}]", type_key(i, params), n.unwrap_or(0)),
        InferType::Vec(i) => format!("Vec[{}]", type_key(i, params)),
        InferType::Rc(i) => format!("Rc[{}]", type_key(i, params)),
        InferType::Ref { referent, mutable } => {
            format!("&{}{}", if *mutable { "mut " } else { "" }, type_key(referent, params))
        }
        other => other.to_string(),
    }
}

pub(crate) fn match_head(
    pat: &InferType,
    params: &[String],
    ty: &InferType,
    out: &mut HashMap<String, InferType>,
) -> bool {
    if let InferType::Struct(n, a) = pat
        && a.is_empty()
        && params.contains(n)
    {
        if let Some(prev) = out.get(n) {
            return prev == ty;
        }
        out.insert(n.clone(), ty.clone());
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
                    .all(|(p, t)| match_head(p, params, t, out))
        }
        (InferType::Tuple(pa), InferType::Tuple(ta)) => {
            pa.len() == ta.len()
                && pa
                    .iter()
                    .zip(ta.iter())
                    .all(|(p, t)| match_head(p, params, t, out))
        }
        (InferType::Array(p, pn), InferType::Array(t, tn)) => {
            pn == tn && match_head(p, params, t, out)
        }
        (InferType::Vec(p), InferType::Vec(t)) | (InferType::Rc(p), InferType::Rc(t)) => {
            match_head(p, params, t, out)
        }
        _ => pat == ty,
    }
}

fn unifiable(
    a: &InferType,
    ap: &[String],
    b: &InferType,
    bp: &[String],
    sa: &mut HashMap<String, InferType>,
    sb: &mut HashMap<String, InferType>,
) -> bool {
    let is_p = |t: &InferType, ps: &[String]| match t {
        InferType::Struct(n, x) if x.is_empty() && ps.contains(n) => Some(n.clone()),
        _ => None,
    };
    if let Some(n) = is_p(a, ap) {
        if let Some(prev) = sa.get(&n).cloned() {
            return unifiable(&prev, &[], b, bp, &mut HashMap::new(), sb);
        }
        sa.insert(n, b.clone());
        return true;
    }
    if let Some(n) = is_p(b, bp) {
        if let Some(prev) = sb.get(&n).cloned() {
            return unifiable(a, ap, &prev, &[], sa, &mut HashMap::new());
        }
        sb.insert(n, a.clone());
        return true;
    }
    match (a, b) {
        (InferType::Struct(x, xa), InferType::Struct(y, ya))
        | (InferType::Enum(x, xa), InferType::Enum(y, ya)) => {
            x == y
                && xa.len() == ya.len()
                && xa
                    .iter()
                    .zip(ya.iter())
                    .all(|(p, q)| unifiable(p, ap, q, bp, sa, sb))
        }
        (InferType::Tuple(xa), InferType::Tuple(ya)) => {
            xa.len() == ya.len()
                && xa
                    .iter()
                    .zip(ya.iter())
                    .all(|(p, q)| unifiable(p, ap, q, bp, sa, sb))
        }
        _ => a == b,
    }
}

fn head_name(ty: &InferType) -> Option<String> {
    match ty {
        InferType::Struct(n, _) | InferType::Enum(n, _) => Some(n.clone()),
        InferType::Tuple(es) => Some(format!("({})", es.len())),
        other => Some(other.to_string()),
    }
}

impl TypeInference {
    pub(crate) fn lookup_for(
        &mut self,
        ty: &InferType,
        member: &str,
        span: Span,
    ) -> super::impls::MethodLookup {
        let mut hits: Vec<MethodEntry> = Vec::new();
        if let Some(key) = head_name(ty)
            && let Some(entries) = self.methods.get(&key)
        {
            hits.extend(entries.iter().filter(|e| e.name == member).cloned());
        }
        hits.extend(self.trait_candidates(ty, member));
        match hits.len() {
            0 => super::impls::MethodLookup::Missing,
            1 => super::impls::MethodLookup::Found(hits.pop().expect("one hit")),
            _ => {
                let names: Vec<String> = hits
                    .iter()
                    .map(|e| format!("`{}`", provider_shown(&e.fn_name)))
                    .collect();
                self.errors.push(TypeError::coded(
                    "E0759",
                    format!(
                        "`{member}` on `{ty}` is provided by both {}; call it by its trait path",
                        names.join(" and ")
                    ),
                    "ambiguous method",
                    span,
                ));
                super::impls::MethodLookup::Ambiguous
            }
        }
    }

    pub(crate) fn trait_path_call(
        &mut self,
        q: &str,
        member: &str,
        args: &[Expr],
        span: Span,
    ) -> (crate::typed_ast::TypedExprKind, InferType) {
        let sig = self
            .trait_defs
            .get(q)
            .and_then(|d| d.methods.iter().find(|m| m.name == member).cloned());
        let Some(sig) = sig else {
            self.errors.push(TypeError::coded(
                "E0755",
                format!("`{member}` is not a method of `{}`", trait_shown(q)),
                "not in the trait",
                span,
            ));
            for a in args {
                self.infer_expr(a);
            }
            return (crate::typed_ast::TypedExprKind::Null, InferType::Dynamic);
        };
        self.note_stub_use(&sig.stub);
        self.infer_call_expr(&ident(&sig.stub, span), args, span)
    }

    pub(crate) fn param_path_call(
        &mut self,
        tp: &str,
        bounds: &TypeBounds,
        member: &str,
        args: &[Expr],
        span: Span,
    ) -> (crate::typed_ast::TypedExprKind, InferType) {
        let Some(entry) = self.bound_method(tp, bounds, member, span) else {
            for a in args {
                self.infer_expr(a);
            }
            return (crate::typed_ast::TypedExprKind::Null, InferType::Dynamic);
        };
        self.note_stub_use(&entry.fn_name);
        let (kind, ty) = self.infer_call_expr(&ident(&entry.fn_name, span), args, span);
        if entry.receiver == Receiver::None {
            self.pin_self_result(&entry.fn_name, &ty, InferType::Struct(tp.to_string(), Vec::new()), span);
        }
        (kind, ty)
    }

    pub(crate) fn pin_self_result(&mut self, stub: &str, got: &InferType, self_ty: InferType, span: Span) {
        let declared = self.env.lookup_function(stub).map(|t| t.as_ref().clone());
        if let Some(InferType::Function { ret, .. }) = declared
            && matches!(ret.as_ref(), InferType::Struct(n, a) if n == SELF && a.is_empty())
        {
            self.constraints.push(crate::constraint::Constraint::equal(
                got.clone(),
                self_ty,
                span,
                crate::constraint::ConstraintReason::Other(
                    "the associated function returns `Self`".to_string(),
                ),
            ));
        }
    }

    pub(crate) fn resolve_trait_name(&self, name: &str) -> Option<String> {
        if let Some(q) = self.trait_names.get(name) {
            return Some(q.clone());
        }
        if let Some((ns, member)) = name.split_once('.')
            && let Some(ex) = self.module_imports.namespaces.get(ns)
            && let Some(def) = ex.traits.get(member)
            && def.is_pub
        {
            return Some(def.qualified.clone());
        }
        if self.trait_defs.contains_key(name) {
            return Some(name.to_string());
        }
        None
    }

    pub(crate) fn is_trait_name(&self, name: &str) -> bool {
        self.trait_names.contains_key(name)
    }

    fn resolve_bounds(&mut self, bounds: &mut [TypeBounds]) {
        for set in bounds.iter_mut() {
            let mut kept = Vec::new();
            for r in std::mem::take(&mut set.traits) {
                match self.resolve_trait_name(&r.name) {
                    Some(q) => kept.push(TraitRef { name: q, span: r.span }),
                    None => self.errors.push(TypeError::coded(
                        "E0106",
                        format!(
                            "no trait named `{}`; a bound is `nogc`, `eq`, `ord`, `Display` or a \
                             trait in scope, combined with `+`",
                            r.name
                        ),
                        "unknown trait",
                        r.span,
                    )),
                }
            }
            set.traits = kept;
        }
    }

    pub(crate) fn module_path(&self) -> String {
        self.module_imports.module_path.clone()
    }

    pub(super) fn collect_traits(&mut self, stmts: Vec<Stmt>) -> Vec<Stmt> {
        let path = self.module_path();
        for stmt in &stmts {
            if let StmtKind::Trait(decl) = &stmt.kind {
                let q = qualify_type_name(&path, &decl.name);
                if self.trait_names.contains_key(&decl.name)
                    || self.type_table.has_struct(&decl.name)
                    || self.type_table.has_enum(&decl.name)
                {
                    self.errors.push(TypeError::coded(
                        "E0757",
                        format!("the name `{}` is already taken by a type or a trait", decl.name),
                        "duplicate name",
                        decl.span,
                    ));
                    continue;
                }
                self.trait_names.insert(decl.name.clone(), q);
            }
        }
        let mut out = Vec::with_capacity(stmts.len());
        for stmt in stmts {
            match stmt.kind {
                StmtKind::Trait(decl) => {
                    for func in self.lower_trait(decl) {
                        let span = func.span;
                        out.push(Stmt::new(StmtKind::Function(func), span));
                    }
                }
                kind => out.push(Stmt::new(kind, stmt.span)),
            }
        }
        for stmt in out.iter_mut() {
            match &mut stmt.kind {
                StmtKind::Function(func) if !func.name.starts_with(STUB) => {
                    let mut b = std::mem::take(&mut func.bounds);
                    self.resolve_bounds(&mut b);
                    func.bounds = b;
                }
                StmtKind::Impl(decl) => {
                    let mut b = std::mem::take(&mut decl.bounds);
                    self.resolve_bounds(&mut b);
                    decl.bounds = b;
                    for m in decl.methods.iter_mut() {
                        let mut b = std::mem::take(&mut m.func.bounds);
                        self.resolve_bounds(&mut b);
                        m.func.bounds = b;
                    }
                }
                _ => {}
            }
        }
        out
    }

    fn lower_trait(&mut self, decl: TraitDecl) -> Vec<Function> {
        let Some(q) = self.trait_names.get(&decl.name).cloned() else {
            return Vec::new();
        };
        let path = self.module_path();
        let mut sigs = Vec::new();
        let mut out = Vec::new();
        let mut seen = HashSet::new();
        for method in decl.methods {
            let mut func = method.func;
            let mname = func.name.clone();
            if !seen.insert(mname.clone()) {
                self.errors.push(TypeError::coded(
                    "E0753",
                    format!("`{mname}` is declared twice in trait `{}`", decl.name),
                    "duplicate trait method",
                    func.span,
                ));
                continue;
            }
            if !func.type_params.is_empty() {
                self.errors.push(TypeError::coded(
                    "E0757",
                    format!(
                        "trait method `{mname}` declares type parameters, which a trait method \
                         cannot do yet"
                    ),
                    "generic trait method",
                    func.span,
                ));
                continue;
            }
            if func.is_nogc {
                self.errors.push(TypeError::coded(
                    "E0751",
                    format!(
                        "trait method `{mname}` is marked `nogc`; the effect of a trait method \
                         is inferred from each impl and cannot be declared"
                    ),
                    "nogc on a trait method",
                    func.span,
                ));
                func.is_nogc = false;
            }
            let in_sig = func
                .params
                .iter()
                .filter_map(|p| p.type_annotation.as_ref())
                .any(mentions_self)
                || func.return_type.as_ref().is_some_and(mentions_self);
            if method.receiver == Receiver::None && !in_sig {
                self.errors.push(TypeError::coded(
                    "E0758",
                    format!(
                        "associated function `{mname}` does not say which type implements it: \
                         `Self` appears nowhere in its signature"
                    ),
                    "Self absent from the signature",
                    func.span,
                ));
                continue;
            }
            let stub = stub_name(&q, &mname);
            let head = self_ann(func.span, None);
            let mut params: Vec<Parameter> = Vec::new();
            if let Some(p) = recv_param(method.receiver, method.mut_self, &head, func.span) {
                params.push(p);
            }
            params.extend(std::mem::take(&mut func.params));
            let body = if method.has_default {
                std::mem::take(&mut func.body)
            } else {
                let args: Vec<Expr> = params.iter().map(|p| ident(&p.name, func.span)).collect();
                let call = Expr::new(
                    ExprKind::Call {
                        callee: Box::new(ident(&stub, func.span)),
                        args,
                    },
                    func.span,
                );
                if func.return_type.is_some() {
                    vec![ret(call, func.span)]
                } else {
                    Vec::new()
                }
            };
            let bounds = vec![TypeBounds {
                traits: vec![TraitRef {
                    name: q.clone(),
                    span: func.span,
                }],
                ..TypeBounds::default()
            }];
            let stub_fn = Function {
                name: stub.clone(),
                type_params: vec![SELF.to_string()],
                bounds,
                params,
                return_type: func.return_type.clone(),
                body,
                decorators: Vec::new(),
                is_pub: true,
                is_nogc: false,
                foreign: None,
                span: func.span,
            };
            self.method_tparams
                .insert(stub.clone(), vec![SELF.to_string()]);
            let saved = std::mem::replace(&mut self.type_params_in_scope, vec![SELF.to_string()]);
            let params_ty: Vec<InferType> = stub_fn
                .params
                .iter()
                .map(|p| match &p.type_annotation {
                    Some(a) => self.type_from_param_annotation(a),
                    None => InferType::Dynamic,
                })
                .collect();
            let ret_ty = match &stub_fn.return_type {
                Some(r) => self.type_from_annotation(r),
                None => InferType::Null,
            };
            self.type_params_in_scope = saved;
            let ty = InferType::Function {
                params: params_ty,
                ret: Box::new(ret_ty),
                nogc: false,
            };
            sigs.push((mname, method.receiver, method.has_default, stub, ty));
            out.push(stub_fn);
        }
        let def = TraitDef {
            name: decl.name.clone(),
            qualified: q.clone(),
            module: path,
            is_pub: decl.is_pub,
            methods: sigs
                .into_iter()
                .map(|(name, receiver, has_default, stub, ty)| TraitMethodSig {
                    name,
                    receiver,
                    has_default,
                    stub,
                    ty,
                })
                .collect(),
        };
        self.local_traits.push(q.clone());
        self.trait_defs.insert(q, def);
        out
    }

    pub(super) fn fill_trait_sigs(&mut self) {
        for q in self.local_traits.clone() {
            let Some(mut def) = self.trait_defs.get(&q).cloned() else {
                continue;
            };
            for m in def.methods.iter_mut() {
                if let Some(ty) = self.env.lookup_function(&m.stub).map(|t| (*t).clone()) {
                    m.ty = (*ty).clone();
                }
            }
            self.trait_defs.insert(q, def);
        }
    }

    pub(super) fn infer_used_builtin_stubs(&mut self) -> Vec<crate::typed_ast::TypedStmt> {
        let mut used: Vec<String> = self.used_builtin_stubs.iter().cloned().collect();
        used.sort();
        let span = Span::dummy();
        let stmts: Vec<Stmt> = used
            .iter()
            .filter_map(|name| {
                let q = name.strip_prefix(STUB)?.split('$').next()?;
                Some(Stmt::new(StmtKind::Function(builtin_stub(q, span)), span))
            })
            .collect();
        self.infer_stmts(&stmts)
    }

    pub(crate) fn note_stub_use(&mut self, name: &str) {
        if let Some(rest) = name.strip_prefix(STUB)
            && let Some(q) = rest.split('$').next()
            && matches!(q, "eq" | "ord" | "Display")
        {
            self.used_builtin_stubs.insert(name.to_string());
        }
    }

    pub(crate) fn structural(&self, q: &str, ty: &InferType) -> bool {
        match q {
            "eq" => ty.satisfies_eq() || self.unit_enum(ty),
            "ord" => ty.satisfies_ord(),
            "Display" => ty.satisfies_eq(),
            _ => false,
        }
    }

    fn unit_enum(&self, ty: &InferType) -> bool {
        matches!(ty, InferType::Enum(n, _) if self.type_table.get_enum(n).is_some_and(|d| d.variants.iter().all(|v| v.data.is_empty())))
    }

    pub(crate) fn impl_for(&self, q: &str, ty: &InferType) -> Option<(ImplDef, HashMap<String, InferType>)> {
        for imp in &self.impl_defs {
            if imp.trait_q != q {
                continue;
            }
            let mut subst = HashMap::new();
            if !match_head(&imp.self_ty, &imp.type_params, ty, &mut subst) {
                continue;
            }
            let ok = imp.type_params.iter().enumerate().all(|(i, tp)| {
                let Some(bound) = imp.bounds.get(i) else {
                    return true;
                };
                let Some(arg) = subst.get(tp) else {
                    return true;
                };
                bound.traits.iter().all(|t| self.satisfies_trait(&t.name, arg))
                    && (!bound.ord || self.satisfies_trait("ord", arg))
                    && (!bound.eq || self.satisfies_trait("eq", arg))
                    && (!bound.nogc || self.type_table.is_nogc_value(arg))
            });
            if ok {
                return Some((imp.clone(), subst));
            }
        }
        None
    }

    pub(crate) fn satisfies_trait(&self, q: &str, ty: &InferType) -> bool {
        if let Some((_, carried)) = self.bounds_on_type_param(ty) {
            return match q {
                "eq" => carried.needs_eq() || carried.traits.iter().any(|t| t.name == q),
                "ord" => carried.needs_ord() || carried.traits.iter().any(|t| t.name == q),
                _ => carried.traits.iter().any(|t| t.name == q),
            };
        }
        if self.structural(q, ty) {
            return true;
        }
        self.impl_for(q, ty).is_some()
    }

    pub(crate) fn impl_method_symbol(&self, q: &str, ty: &InferType, method: &str) -> Option<String> {
        let (imp, _) = self.impl_for(q, ty)?;
        imp.methods
            .iter()
            .find(|(m, _)| m == method)
            .map(|(_, s)| s.clone())
    }

    pub(super) fn lower_trait_impl(&mut self, decl: ImplDecl, out: &mut Vec<Function>) {
        let trait_ref = decl.trait_ref.clone().expect("trait impl");
        if trait_ref.name == "nogc" {
            self.errors.push(TypeError::coded(
                "E0751",
                "`nogc` is a bound the compiler derives from a type's fields; it cannot be \
                 implemented"
                    .to_string(),
                "not a trait",
                trait_ref.span,
            ));
            return;
        }
        let Some(q) = self.resolve_trait_name(&trait_ref.name) else {
            self.errors.push(TypeError::coded(
                "E0106",
                format!("no trait named `{}`", trait_ref.name),
                "unknown trait",
                trait_ref.span,
            ));
            return;
        };
        let Some(tdef) = self.trait_defs.get(&q).cloned() else {
            return;
        };
        let tshown = trait_shown(&q).to_string();
        let head = decl.self_ty.clone();
        if head.reference.is_some() || head.is_slice || head.is_function_type() {
            self.errors.push(TypeError::coded(
                "E0757",
                format!("an `impl {tshown} for ..` names a struct, an enum or a builtin value type"),
                "not a nominal type",
                head.span,
            ));
            return;
        }
        if decl.type_params.iter().any(|p| *p == head.name)
            && head.type_param.is_none()
            && head.type_params.is_empty()
        {
            self.errors.push(TypeError::coded(
                "E0757",
                format!(
                    "`impl<{}> {tshown} for {}` implements a trait for a bare type parameter, \
                     which would cover every type; implement it for each type",
                    head.name, head.name
                ),
                "bare type parameter",
                head.span,
            ));
            return;
        }
        let mut used = HashSet::new();
        collect_ann_names(&head, &mut used);
        if let Some(tp) = decl.type_params.iter().find(|tp| !used.contains(*tp)) {
            self.errors.push(TypeError::coded(
                "E0757",
                format!(
                    "the type parameter `{tp}` of this `impl` does not appear in `{}`",
                    super::impls::annotation_shown(&head)
                ),
                "unconstrained type parameter",
                decl.span,
            ));
            return;
        }
        let saved = std::mem::replace(&mut self.type_params_in_scope, decl.type_params.clone());
        let errs_before = self.errors.len();
        let self_ty = self.type_from_annotation(&head);
        self.type_params_in_scope = saved;
        if self.errors.len() > errs_before || matches!(self_ty, InferType::Dynamic) {
            return;
        }
        let shown_ty = super::impls::annotation_shown(&head);
        if self.structural(&q, &self_ty) {
            self.errors.push(TypeError::coded(
                "E0753",
                format!("`{shown_ty}` already implements `{tshown}` structurally; it cannot be re-implemented"),
                "already implemented",
                head.span,
            ));
            return;
        }
        let local_trait = self.local_traits.contains(&q);
        let local_type = match &self_ty {
            InferType::Struct(n, _) | InferType::Enum(n, _) => {
                !self.imported_types.contains(n)
                    && !n.contains('.')
                    && (self.type_table.has_struct(n) || self.type_table.has_enum(n))
                    && !matches!(source_type_name(n), "Option" | "Result")
            }
            _ => false,
        };
        if !local_trait && !local_type {
            self.errors.push(TypeError::coded(
                "E0757",
                format!(
                    "`impl {tshown} for {shown_ty}` must live in the module that declares \
                     `{tshown}` or the one that declares `{shown_ty}`"
                ),
                "orphan impl",
                decl.span,
            ));
            return;
        }
        for other in self.impl_defs.iter().filter(|i| i.trait_q == q) {
            let mut sa = HashMap::new();
            let mut sb = HashMap::new();
            if unifiable(&self_ty, &decl.type_params, &other.self_ty, &other.type_params, &mut sa, &mut sb) {
                let same = type_key(&self_ty, &decl.type_params)
                    == type_key(&other.self_ty, &other.type_params);
                let message = if same {
                    format!("`{tshown}` is implemented twice for `{shown_ty}`")
                } else {
                    format!(
                        "this `impl {tshown} for {shown_ty}` overlaps another impl of `{tshown}` \
                         for `{}`; the impls of one trait must not overlap",
                        other.self_ty
                    )
                };
                self.errors.push(TypeError::coded("E0753", message, "overlapping impl", decl.span));
                return;
            }
        }
        let fields: Vec<String> = match &self_ty {
            InferType::Struct(n, _) => self
                .type_table
                .get_struct(n)
                .map(|d| d.fields.iter().map(|f| f.name.clone()).collect())
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        let key = head_name(&self_ty);
        let path = self.module_path();
        let owned: HashSet<String> = self.local_type_names();
        let qualified_self = qualify_infer_type(&self_ty, &path, &owned);
        let tkey = type_key(&qualified_self, &decl.type_params);
        let mut errored = false;
        for m in &tdef.methods {
            if fields.contains(&m.name) {
                self.errors.push(TypeError::coded(
                    "E0342",
                    format!(
                        "`{shown_ty}` has a field named `{}`; the method `{}` of `{tshown}` \
                         cannot share it",
                        m.name, m.name
                    ),
                    "clashes with a field",
                    decl.span,
                ));
                errored = true;
            }
            if let Some(k) = &key
                && self
                    .methods
                    .get(k)
                    .is_some_and(|es| es.iter().any(|e| e.name == m.name))
            {
                self.errors.push(TypeError::coded(
                    "E0759",
                    format!(
                        "`{}` on `{shown_ty}` is provided by both its own `impl` and \
                         `impl {tshown} for {shown_ty}`",
                        m.name
                    ),
                    "ambiguous method",
                    decl.span,
                ));
                errored = true;
            }
        }
        let mut provided: Vec<(String, String)> = Vec::new();
        for method in decl.methods {
            let mut func = method.func;
            let mname = func.name.clone();
            let Some(sig) = tdef.methods.iter().find(|m| m.name == mname) else {
                self.errors.push(TypeError::coded(
                    "E0755",
                    format!("`{mname}` is not a method of `{tshown}`"),
                    "not in the trait",
                    func.span,
                ));
                errored = true;
                continue;
            };
            if provided.iter().any(|(n, _)| *n == mname) {
                self.errors.push(TypeError::coded(
                    "E0753",
                    format!("`{mname}` is defined twice in `impl {tshown} for {shown_ty}`"),
                    "duplicate method",
                    func.span,
                ));
                errored = true;
                continue;
            }
            if !func.type_params.is_empty() || method.receiver != sig.receiver {
                self.signature_mismatch(&mname, &tshown, func.span);
                errored = true;
                continue;
            }
            let symbol = format!("{IMPL}{q}${tkey}${mname}");
            let mut params = Vec::with_capacity(func.params.len() + 1);
            if let Some(p) = recv_param(method.receiver, method.mut_self, &head, func.span) {
                params.push(p);
            }
            for mut p in std::mem::take(&mut func.params) {
                if let Some(ann) = p.type_annotation.as_mut() {
                    super::impls::subst_ann(ann, &head);
                }
                params.push(p);
            }
            func.params = params;
            if let Some(r) = func.return_type.as_mut() {
                super::impls::subst_ann(r, &head);
            }
            super::impls::subst_stmts(&mut func.body, &head);
            if !self.impl_sig_matches(&func, sig, &self_ty, &decl.type_params) {
                self.signature_mismatch(&mname, &tshown, func.span);
                errored = true;
                continue;
            }
            func.type_params = decl.type_params.clone();
            func.bounds = decl.bounds.clone();
            func.bounds.resize(decl.type_params.len(), TypeBounds::default());
            func.name = symbol.clone();
            func.is_pub = true;
            self.method_tparams
                .insert(symbol.clone(), decl.type_params.clone());
            provided.push((mname, symbol));
            out.push(func);
        }
        let missing: Vec<String> = tdef
            .methods
            .iter()
            .filter(|m| !m.has_default && !provided.iter().any(|(n, _)| *n == m.name))
            .map(|m| format!("`{}`", m.name))
            .collect();
        if !missing.is_empty() && !errored {
            self.errors.push(TypeError::coded(
                "E0752",
                format!(
                    "`impl {tshown} for {shown_ty}` is missing {}",
                    missing.join(", ")
                ),
                "missing trait method",
                decl.span,
            ));
            errored = true;
        }
        if errored {
            return;
        }
        let imp = ImplDef {
            trait_q: q.clone(),
            type_params: decl.type_params.clone(),
            bounds: decl.bounds.clone(),
            self_ty: self_ty.clone(),
            methods: provided,
            module: path,
            span: decl.span,
            values: Vec::new(),
        };
        self.register_impl_entries(&imp);
        self.local_impls.push(self.impl_defs.len());
        self.impl_defs.push(imp);
    }

    fn signature_mismatch(&mut self, m: &str, t: &str, span: Span) {
        self.errors.push(TypeError::coded(
            "E0754",
            format!("the signature of `{m}` does not match the trait `{t}`"),
            "signature differs from the trait",
            span,
        ));
    }

    fn impl_sig_matches(
        &mut self,
        func: &Function,
        sig: &TraitMethodSig,
        self_ty: &InferType,
        tparams: &[String],
    ) -> bool {
        let InferType::Function { params, ret, .. } = &sig.ty else {
            return true;
        };
        if params.len() != func.params.len() {
            return false;
        }
        let saved = std::mem::replace(&mut self.type_params_in_scope, tparams.to_vec());
        let mut ok = true;
        for (p, want) in func.params.iter().zip(params.iter()) {
            let Some(ann) = p.type_annotation.as_ref() else {
                ok = false;
                break;
            };
            let got = self.type_from_param_annotation(ann);
            if got != subst_self(want, self_ty) {
                ok = false;
                break;
            }
        }
        if ok {
            let got = match &func.return_type {
                Some(r) => self.type_from_annotation(r),
                None => InferType::Null,
            };
            let want = subst_self(ret, self_ty);
            ok = got == want || (func.return_type.is_none() && matches!(want, InferType::Null));
        }
        self.type_params_in_scope = saved;
        ok
    }

    pub(crate) fn register_impl_entries(&mut self, imp: &ImplDef) {
        let Some(key) = head_name(&imp.self_ty) else {
            return;
        };
        let Some(tdef) = self.trait_defs.get(&imp.trait_q).cloned() else {
            return;
        };
        let entries = self.trait_methods.entry(key).or_default();
        for m in &tdef.methods {
            let fn_name = imp
                .methods
                .iter()
                .find(|(n, _)| *n == m.name)
                .map(|(_, s)| s.clone())
                .unwrap_or_else(|| m.stub.clone());
            entries.push(MethodEntry {
                name: m.name.clone(),
                receiver: m.receiver,
                fn_name,
                is_pub: true,
                module: String::new(),
                value: None,
                head: Some((imp.type_params.clone(), imp.self_ty.clone())),
            });
        }
    }

    pub(super) fn check_ord_has_eq(&mut self) {
        let local: Vec<ImplDef> = self.local_impls.iter().map(|&i| self.impl_defs[i].clone()).collect();
        for imp in local.iter().filter(|i| i.trait_q == "ord") {
            let has_eq = self.impl_defs.iter().any(|o| {
                o.trait_q == "eq"
                    && type_key(&o.self_ty, &o.type_params) == type_key(&imp.self_ty, &imp.type_params)
            });
            if !has_eq {
                let shown = imp.self_ty.to_string();
                self.errors.push(TypeError::coded(
                    "E0752",
                    format!("`impl ord for {shown}` is missing `impl eq for {shown}`; `ord` requires `eq`"),
                    "ord without eq",
                    imp.span,
                ));
            }
        }
    }

    pub(crate) fn local_type_names(&self) -> HashSet<String> {
        self.type_table
            .nominal_names()
            .filter(|n| !self.imported_types.contains(*n) && !n.contains('.'))
            .cloned()
            .collect()
    }

    pub(crate) fn trait_candidates(&self, ty: &InferType, member: &str) -> Vec<MethodEntry> {
        let mut out = Vec::new();
        if !matches!(ty, InferType::Struct(..) | InferType::Enum(..) | InferType::Var(_)) {
            for q in ["eq", "ord", "Display"] {
                if !self.structural(q, ty) {
                    continue;
                }
                if let Some(def) = self.trait_defs.get(q)
                    && let Some(m) = def.methods.iter().find(|m| m.name == member)
                {
                    out.push(MethodEntry {
                        name: m.name.clone(),
                        receiver: m.receiver,
                        fn_name: m.stub.clone(),
                        is_pub: true,
                        module: String::new(),
                        value: None,
                        head: None,
                    });
                }
            }
        }
        let Some(key) = head_name(ty) else {
            return out;
        };
        let Some(entries) = self.trait_methods.get(&key) else {
            return out;
        };
        out.extend(entries
            .iter()
            .filter(|e| e.name == member)
            .filter(|e| match &e.head {
                Some((tps, pat)) => match_head_loose(pat, tps, ty),
                None => true,
            })
            .cloned());
        out
    }

    pub(crate) fn bound_method(
        &mut self,
        tp: &str,
        bounds: &TypeBounds,
        member: &str,
        span: Span,
    ) -> Option<MethodEntry> {
        let mut traits: Vec<String> = bounds.traits.iter().map(|t| t.name.clone()).collect();
        if bounds.needs_eq() {
            traits.push("eq".to_string());
        }
        if bounds.needs_ord() {
            traits.push("ord".to_string());
        }
        let mut hits: Vec<(String, TraitMethodSig)> = Vec::new();
        for q in &traits {
            if let Some(def) = self.trait_defs.get(q)
                && let Some(m) = def.methods.iter().find(|m| m.name == member)
            {
                hits.push((q.clone(), m.clone()));
            }
        }
        match hits.len() {
            0 => {
                let listing = if traits.is_empty() {
                    format!("`{tp}` has no bound")
                } else {
                    format!(
                        "its bounds are {}",
                        traits
                            .iter()
                            .map(|q| format!("`{}`", trait_shown(q)))
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                };
                self.errors.push(TypeError::coded(
                    "E0756",
                    format!("`{tp}` has no method `{member}`; {listing}"),
                    "method outside the bounds",
                    span,
                ));
                None
            }
            1 => {
                let (_, m) = hits.pop().expect("one hit");
                Some(MethodEntry {
                    name: m.name.clone(),
                    receiver: m.receiver,
                    fn_name: m.stub.clone(),
                    is_pub: true,
                    module: String::new(),
                    value: None,
                    head: None,
                })
            }
            _ => {
                self.errors.push(TypeError::coded(
                    "E0759",
                    format!(
                        "`{member}` on `{tp}` is provided by both {}; call it as \
                         `Trait::{member}(..)`",
                        hits.iter()
                            .map(|(q, _)| format!("`{}`", trait_shown(q)))
                            .collect::<Vec<_>>()
                            .join(" and ")
                    ),
                    "ambiguous method",
                    span,
                ));
                None
            }
        }
    }

    pub(crate) fn define_trait_stub_values(&mut self, def: &TraitDef) {
        for m in &def.methods {
            if self.env.lookup_function(&m.stub).is_some() {
                self.method_tparams
                    .insert(m.stub.clone(), vec![SELF.to_string()]);
                continue;
            }
            let value = ModuleValue {
                kind: ItemKind::Function,
                ty: m.ty.clone(),
                qualified: m.stub.clone(),
                is_pub: true,
                type_params: vec![SELF.to_string()],
                bounds: vec![TypeBounds {
                    traits: vec![TraitRef {
                        name: def.qualified.clone(),
                        span: Span::dummy(),
                    }],
                    ..TypeBounds::default()
                }],
                literal: None,
            };
            self.method_tparams
                .insert(m.stub.clone(), vec![SELF.to_string()]);
            self.define_imported_value(&value);
        }
    }
}

fn match_head_loose(pat: &InferType, params: &[String], ty: &InferType) -> bool {
    fn go(pat: &InferType, params: &[String], ty: &InferType, out: &mut HashMap<String, InferType>) -> bool {
        if matches!(ty, InferType::Var(_) | InferType::Dynamic) {
            return true;
        }
        if let InferType::Struct(n, a) = pat
            && a.is_empty()
            && params.contains(n)
        {
            if let Some(prev) = out.get(n) {
                return prev == ty || !ty.is_concrete() || !prev.is_concrete();
            }
            out.insert(n.clone(), ty.clone());
            return true;
        }
        match (pat, ty) {
            (InferType::Struct(a, pa), InferType::Struct(b, ta))
            | (InferType::Enum(a, pa), InferType::Enum(b, ta)) => {
                a == b && pa.len() == ta.len() && pa.iter().zip(ta.iter()).all(|(p, t)| go(p, params, t, out))
            }
            (InferType::Tuple(pa), InferType::Tuple(ta)) => {
                pa.len() == ta.len() && pa.iter().zip(ta.iter()).all(|(p, t)| go(p, params, t, out))
            }
            _ => pat == ty,
        }
    }
    go(pat, params, ty, &mut HashMap::new())
}

pub(crate) fn subst_self(ty: &InferType, self_ty: &InferType) -> InferType {
    let go = |t: &InferType| subst_self(t, self_ty);
    match ty {
        InferType::Struct(n, a) if n == SELF && a.is_empty() => self_ty.clone(),
        InferType::Struct(n, a) => InferType::Struct(n.clone(), a.iter().map(go).collect()),
        InferType::Enum(n, a) => InferType::Enum(n.clone(), a.iter().map(go).collect()),
        InferType::Tuple(a) => InferType::Tuple(a.iter().map(go).collect()),
        InferType::Array(i, n) => InferType::Array(Box::new(go(i)), *n),
        InferType::Vec(i) => InferType::Vec(Box::new(go(i))),
        InferType::Rc(i) => InferType::Rc(Box::new(go(i))),
        InferType::Ref { referent, mutable } => InferType::Ref {
            referent: Box::new(go(referent)),
            mutable: *mutable,
        },
        InferType::Slice { elem, mutable } => InferType::Slice {
            elem: Box::new(go(elem)),
            mutable: *mutable,
        },
        InferType::Function { params, ret, nogc } => InferType::Function {
            params: params.iter().map(go).collect(),
            ret: Box::new(go(ret)),
            nogc: *nogc,
        },
        other => other.clone(),
    }
}

fn collect_ann_names(ann: &TypeAnnotation, out: &mut HashSet<String>) {
    out.insert(ann.name.clone());
    if let Some(p) = &ann.type_param {
        collect_ann_names(p, out);
    }
    for p in &ann.type_params {
        collect_ann_names(p, out);
    }
    if let Some(es) = &ann.tuple_elems {
        for e in es {
            collect_ann_names(e, out);
        }
    }
}


fn typed_ref(e: crate::typed_ast::TypedExpr) -> crate::typed_ast::TypedExpr {
    let span = e.span;
    let ty = InferType::Ref {
        referent: Box::new(e.ty.clone()),
        mutable: false,
    };
    crate::typed_ast::TypedExpr::new(
        crate::typed_ast::TypedExprKind::Reference {
            mutable: false,
            operand: Box::new(e),
            receiver_of: None,
        },
        ty,
        span,
    )
}

fn bind_operands(
    operands: Vec<crate::typed_ast::TypedExpr>,
) -> (Vec<crate::typed_ast::TypedStmt>, Vec<crate::typed_ast::TypedExpr>) {
    use crate::typed_ast::{TypedExpr, TypedExprKind, TypedStmt, TypedStmtKind};
    let mut stmts = Vec::new();
    let mut refs = Vec::new();
    for (k, e) in operands.into_iter().enumerate() {
        if crate::place_spine::denotes_a_place(&e) {
            refs.push(typed_ref(e));
            continue;
        }
        let span = e.span;
        let name = format!("__trt${}_{}_{k}", span.start, span.end);
        let ty = e.ty.clone();
        stmts.push(TypedStmt {
            kind: TypedStmtKind::Let {
                name: name.clone(),
                mutable: false,
                initializer: e,
                var_type: ty.clone(),
                is_pub: false,
            },
            span,
        });
        refs.push(typed_ref(TypedExpr::new(TypedExprKind::Identifier(name), ty, span)));
    }
    (stmts, refs)
}

fn wrap_block(
    stmts: Vec<crate::typed_ast::TypedStmt>,
    tail: crate::typed_ast::TypedExpr,
) -> crate::typed_ast::TypedExpr {
    use crate::typed_ast::{TypedExpr, TypedExprKind};
    if stmts.is_empty() {
        return tail;
    }
    let span = tail.span;
    let ty = tail.ty.clone();
    TypedExpr::new(
        TypedExprKind::Block {
            stmts,
            tail: Box::new(tail),
        },
        ty,
        span,
    )
}

fn typed_call(
    sym: &str,
    args: Vec<crate::typed_ast::TypedExpr>,
    ret: InferType,
    span: Span,
) -> crate::typed_ast::TypedExpr {
    use crate::typed_ast::{TypedExpr, TypedExprKind};
    let fty = InferType::Function {
        params: args.iter().map(|a| a.ty.clone()).collect(),
        ret: Box::new(ret.clone()),
        nogc: false,
    };
    TypedExpr::new(
        TypedExprKind::Call {
            callee: Box::new(TypedExpr::new(
                TypedExprKind::Identifier(sym.to_string()),
                fty,
                span,
            )),
            args,
        },
        ret,
        span,
    )
}

impl TypeInference {
    fn display_call(
        &self,
        e: crate::typed_ast::TypedExpr,
        at: Span,
    ) -> crate::typed_ast::TypedExpr {
        if !matches!(e.ty, InferType::Struct(..) | InferType::Enum(..)) {
            return e;
        }
        let Some(sym) = self.impl_method_symbol("Display", &e.ty, "to_string") else {
            return e;
        };
        let (stmts, refs) = bind_operands(vec![e]);
        wrap_block(stmts, typed_call(&sym, refs, InferType::String, at))
    }

    pub(super) fn desugar_trait_ops(
        &self,
        expr: crate::typed_ast::TypedExpr,
    ) -> crate::typed_ast::TypedExpr {
        use crate::typed_ast::{TypedExpr, TypedExprKind, TypedFmtStringPart};
        let span = expr.span;
        match expr.kind {
            TypedExprKind::Binary { left, op, right }
                if matches!(
                    op,
                    BinaryOp::Eq
                        | BinaryOp::Ne
                        | BinaryOp::Lt
                        | BinaryOp::Le
                        | BinaryOp::Gt
                        | BinaryOp::Ge
                ) && matches!(left.ty, InferType::Struct(..) | InferType::Enum(..)) =>
            {
                let (q, m, ret) = if matches!(op, BinaryOp::Eq | BinaryOp::Ne) {
                    ("eq", "eq", InferType::Bool)
                } else {
                    ("ord", "cmp", InferType::I64)
                };
                let Some(sym) = self.impl_method_symbol(q, &left.ty, m) else {
                    return TypedExpr::new(TypedExprKind::Binary { left, op, right }, expr.ty, span);
                };
                let (stmts, refs) = bind_operands(vec![*left, *right]);
                let call = typed_call(&sym, refs, ret, span);
                let kind = match op {
                    BinaryOp::Eq => return wrap_block(stmts, call),
                    BinaryOp::Ne => TypedExprKind::Unary {
                        op: UnaryOp::Not,
                        operand: Box::new(call),
                    },
                    cmp => TypedExprKind::Binary {
                        left: Box::new(call),
                        op: cmp,
                        right: Box::new(TypedExpr::new(TypedExprKind::Int(0), InferType::I64, span)),
                    },
                };
                wrap_block(stmts, TypedExpr::new(kind, InferType::Bool, span))
            }
            TypedExprKind::Call { callee, mut args }
                if matches!(&callee.kind, TypedExprKind::Identifier(n) if n == "print" || n == "println")
                    && args.len() == 1 =>
            {
                let a = args.pop().expect("one argument");
                args.push(self.display_call(a, span));
                TypedExpr::new(TypedExprKind::Call { callee, args }, expr.ty, span)
            }
            TypedExprKind::FmtString(parts) => {
                let parts = parts
                    .into_iter()
                    .map(|p| match p {
                        TypedFmtStringPart::Expr(e) => {
                            TypedFmtStringPart::Expr(Box::new(self.display_call(*e, span)))
                        }
                        other => other,
                    })
                    .collect();
                TypedExpr::new(TypedExprKind::FmtString(parts), expr.ty, span)
            }
            kind => TypedExpr::new(kind, expr.ty, span),
        }
    }
}

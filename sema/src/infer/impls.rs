use super::TypeInference;
use crate::constraint::TypeError;
use crate::modules::MethodEntry;
use crate::types::InferType;
use aelys_syntax::{
    CatchHandler, Expr, ExprKind, FmtStringPart, Function, ImplDecl, MatchArm, Parameter, Pattern,
    Receiver, RefKind, Span, Stmt, StmtKind, TypeAnnotation, TypeBounds,
};
use std::collections::HashSet;

const BUILTIN_HEADS: &[&str] = &["Vec", "Rc", "Option", "Result", "Never", "Self"];
pub(crate) const SLICE_KEY: &str = "[]";
const TRAIT_METHOD_NAMES: &[&str] = &["eq", "cmp", "to_string"];

pub(crate) fn builtin_head_key(head: &TypeAnnotation) -> Option<&'static str> {
    if head.reference.is_some() || head.is_function_type() || head.array_size.is_some() {
        return None;
    }
    if head.is_slice {
        return Some(SLICE_KEY);
    }
    match head.name.as_str() {
        "string" | "str" | "String" => Some("string"),
        "char" => Some("char"),
        "i64" => Some("i64"),
        "f64" => Some("f64"),
        "Vec" => Some("Vec"),
        _ => None,
    }
}

pub(crate) fn builtin_type_key(ty: &InferType) -> Option<&'static str> {
    match ty {
        InferType::String | InferType::StrRef => Some("string"),
        InferType::Char => Some("char"),
        InferType::I64 => Some("i64"),
        InferType::F64 => Some("f64"),
        InferType::Vec(_) => Some("Vec"),
        InferType::Slice { .. } | InferType::Array(..) => Some(SLICE_KEY),
        _ => None,
    }
}

pub(crate) fn builtin_fn_prefix(key: &str) -> &str {
    if key == SLICE_KEY { "slice" } else { key }
}

pub fn builtin_key_of_prefix(prefix: &str) -> Option<&'static str> {
    match prefix {
        "string" | "str" | "String" => Some("string"),
        "char" => Some("char"),
        "i64" => Some("i64"),
        "f64" => Some("f64"),
        "Vec" => Some("Vec"),
        "slice" => Some(SLICE_KEY),
        _ => None,
    }
}

pub fn is_builtin_key(key: &str) -> bool {
    matches!(key, "string" | "char" | "i64" | "f64" | "Vec") || key == SLICE_KEY
}

pub(crate) fn builtin_key_shown(key: &str) -> &str {
    if key == SLICE_KEY { "[T]" } else { key }
}

fn builtin_name_clash(key: &str, name: &str) -> Option<String> {
    let shown = builtin_key_shown(key);
    if TRAIT_METHOD_NAMES.contains(&name) {
        return Some(format!(
            "`{name}` is a method of a builtin trait; a method of `{shown}` cannot share its name"
        ));
    }
    if key == "Vec"
        && super::expr::intrinsic_method_names("Vec").is_some_and(|names| names.contains(&name))
    {
        return Some(format!("`{name}` is an intrinsic method of `Vec`"));
    }
    if name == "len" && matches!(key, "string" | "Vec" | "[]") {
        return Some(format!("`len` is an intrinsic method of `{shown}`"));
    }
    None
}

pub(crate) enum MethodLookup {
    Found(MethodEntry),
    Ambiguous,
    Missing,
}

impl TypeInference {
    pub(super) fn collect_impls(&mut self, stmts: Vec<Stmt>) -> Vec<Stmt> {
        let mut out = Vec::with_capacity(stmts.len());
        let mut trait_impls = Vec::new();
        for stmt in stmts {
            match stmt.kind {
                StmtKind::Impl(decl) if decl.trait_ref.is_some() => trait_impls.push(decl),
                StmtKind::Impl(decl) => {
                    for func in self.lower_impl(decl) {
                        let span = func.span;
                        out.push(Stmt::new(StmtKind::Function(func), span));
                    }
                }
                kind => out.push(Stmt::new(kind, stmt.span)),
            }
        }
        for decl in trait_impls {
            let mut funcs = Vec::new();
            self.lower_trait_impl(decl, &mut funcs);
            for func in funcs {
                let span = func.span;
                out.push(Stmt::new(StmtKind::Function(func), span));
            }
        }
        out
    }

    pub(crate) fn in_std(&self) -> bool {
        self.module_imports.std_privilege || self.module_path().starts_with("std.")
    }

    fn impl_head_error(&mut self, decl: &ImplDecl) -> bool {
        let head = &decl.self_ty;
        let name = head.name.as_str();
        if self.in_std() && builtin_head_key(head).is_some() {
            return self.unconstrained_impl_param(decl);
        }
        if head.reference.is_some() || head.is_slice || head.is_function_type() {
            self.errors.push(TypeError::coded(
                "E0341",
                "an `impl` block names a struct or an enum declared in this module".to_string(),
                "not a nominal type",
                head.span,
            ));
            return true;
        }
        let lower = name.to_lowercase();
        let std_declared = self.in_std() && self.declares_local(name);
        if (BUILTIN_HEADS.contains(&name) && !std_declared)
            || (super::KNOWN_TYPE_NAMES.contains(&lower.as_str()) && !self.declares_local(name))
        {
            let shown = if name == "Option" || name == "Result" {
                format!(
                    "cannot write an `impl` for `{name}`: `{name}` is a prelude type, and methods \
                     go in the module that declares the type"
                )
            } else {
                format!(
                    "cannot write an `impl` for `{name}`: `{name}` is a builtin type, and methods \
                     go in the module that declares the type"
                )
            };
            self.errors
                .push(TypeError::coded("E0341", shown, "builtin type", head.span));
            return true;
        }
        if let Some(qualified) = self.crate_type_of_other_module(head) {
            let declared = self.declared_type_params(&qualified);
            let found = if !head.type_params.is_empty() {
                head.type_params.len()
            } else {
                usize::from(head.type_param.is_some())
            };
            if declared != found {
                let shown = crate::modules::strip_type_head(&qualified);
                self.errors.push(TypeError::type_arg_count(
                    "struct", shown, declared, found, head.span,
                ));
                return true;
            }
            return self.unconstrained_impl_param(decl);
        }
        let foreign = if name.contains('.') {
            Some(name.to_string())
        } else {
            self.import_aliases.get(name).cloned()
        };
        if let Some(qualified) = foreign {
            let bare = crate::modules::strip_type_head(&qualified);
            let module = bare.rsplit_once('.').map(|(m, _)| m.to_string());
            let src = crate::modules::source_type_name(&qualified).to_string();
            let message = match module {
                Some(m) => format!(
                    "cannot write an `impl` for `{src}`: `{src}` is declared in module `{m}`; \
                     methods go in the module that declares the type"
                ),
                None => format!(
                    "cannot write an `impl` for `{src}`: `{src}` is declared in another module; \
                     methods go in the module that declares the type"
                ),
            };
            self.errors.push(TypeError::coded(
                "E0341",
                message,
                "foreign type",
                head.span,
            ));
            return true;
        }
        if !self.declares_local(name) {
            self.errors.push(TypeError::coded(
                "E0306",
                format!("unknown type `{name}`"),
                "unknown type",
                head.span,
            ));
            return true;
        }
        let declared = self.declared_type_params(name);
        let found = if !head.type_params.is_empty() {
            head.type_params.len()
        } else {
            usize::from(head.type_param.is_some())
        };
        if declared != found {
            self.errors.push(TypeError::type_arg_count(
                "struct", name, declared, found, head.span,
            ));
            return true;
        }
        self.unconstrained_impl_param(decl)
    }

    fn unconstrained_impl_param(&mut self, decl: &ImplDecl) -> bool {
        let head = &decl.self_ty;
        let mut used = HashSet::new();
        collect_names(head, &mut used);
        for tp in &decl.type_params {
            if !used.contains(tp) {
                self.errors.push(TypeError::coded(
                    "E0340",
                    format!(
                        "the type parameter `{tp}` of this `impl` does not appear in `{}`",
                        annotation_shown(head)
                    ),
                    "unconstrained type parameter",
                    decl.span,
                ));
                return true;
            }
        }
        false
    }

    // rust keeps inherent impls inside the crate of the type; every module outside std is one crate
    fn crate_type_of_other_module(&self, head: &TypeAnnotation) -> Option<String> {
        if self.in_std() {
            return None;
        }
        let qualified = if head.name.contains('.') {
            let (ns, member) = head.name.split_once('.')?;
            let exports = self.module_imports.namespaces.get(ns)?;
            exports.types.get(member)?.qualified.clone()
        } else {
            self.imported_type_name(&head.name)?
        };
        let module = crate::modules::module_of_type(&qualified)?;
        (!crate::modules::is_std_module(module)).then_some(qualified)
    }

    fn declares_local(&self, name: &str) -> bool {
        !self.imported_types.contains(name)
            && (self.type_table.has_struct(name) || self.type_table.has_enum(name))
    }

    fn declared_type_params(&self, name: &str) -> usize {
        if let Some(def) = self.type_table.get_struct(name) {
            return def.type_params.len();
        }
        self.type_table
            .get_enum(name)
            .map_or(0, |d| d.type_params.len())
    }

    fn lower_impl(&mut self, decl: ImplDecl) -> Vec<Function> {
        if self.impl_head_error(&decl) {
            return Vec::new();
        }
        let head = decl.self_ty.clone();
        let builtin = if self.in_std() {
            builtin_head_key(&head)
        } else {
            None
        };
        let foreign = self.crate_type_of_other_module(&head);
        let table_key = match (builtin, &foreign) {
            (Some(key), _) => key.to_string(),
            (None, Some(qualified)) => qualified.clone(),
            (None, None) => head.name.clone(),
        };
        let type_name = match (builtin, &foreign) {
            (Some(key), _) => builtin_fn_prefix(key).to_string(),
            (None, Some(qualified)) => qualified.clone(),
            (None, None) => head.name.clone(),
        };
        let shown = crate::modules::strip_type_head(&type_name);
        let fn_prefix = match &foreign {
            Some(qualified) => crate::modules::strip_type_head(qualified).replace('.', "$"),
            None => type_name.clone(),
        };
        let fields: Vec<String> = self
            .type_table
            .get_struct(&type_name)
            .map(|d| d.fields.iter().map(|f| f.name.clone()).collect())
            .unwrap_or_default();
        let variants: Vec<String> = self
            .type_table
            .get_enum(&type_name)
            .map(|d| d.variants.iter().map(|v| v.name.clone()).collect())
            .unwrap_or_default();

        let mut out = Vec::new();
        for method in decl.methods {
            let mut func = method.func;
            let mname = func.name.clone();
            if fields.contains(&mname) {
                self.errors.push(TypeError::coded(
                    "E0342",
                    format!("`{shown}` has a field named `{mname}`; a method cannot share it"),
                    "clashes with a field",
                    func.span,
                ));
                continue;
            }
            if variants.contains(&mname) {
                self.errors.push(TypeError::coded(
                    "E0342",
                    format!(
                        "`{mname}` is already a variant of `{shown}`; an associated function \
                         cannot share its name"
                    ),
                    "clashes with a variant",
                    func.span,
                ));
                continue;
            }
            if let Some(key) = builtin
                && let Some(message) = builtin_name_clash(key, &mname)
            {
                self.errors.push(TypeError::coded(
                    "E0342",
                    message,
                    "clashes with a builtin method",
                    func.span,
                ));
                continue;
            }
            if builtin == Some(SLICE_KEY) && method.receiver == Receiver::Value {
                self.errors.push(TypeError::coded(
                    "E0341",
                    format!("a slice method takes `&self` or `&mut self`; `{mname}` takes `self`"),
                    "slice method by value",
                    func.span,
                ));
                continue;
            }
            let shown_head = match builtin {
                Some(key) => builtin_key_shown(key).to_string(),
                None => shown.to_string(),
            };
            let table_is_shared = foreign.is_some() || builtin.is_some();
            let table = if builtin.is_some() {
                &mut self.builtin_methods
            } else {
                &mut self.methods
            };
            let entries = table.entry(table_key.clone()).or_default();
            if entries
                .iter()
                .any(|e| e.name == mname && (table_is_shared || e.module.is_empty()))
            {
                self.errors.push(TypeError::coded(
                    "E0342",
                    format!("duplicate method `{mname}` on `{shown_head}`"),
                    "already defined",
                    func.span,
                ));
                continue;
            }
            if let Some(tp) = func
                .type_params
                .iter()
                .find(|tp| decl.type_params.contains(tp))
            {
                self.errors.push(TypeError::coded(
                    "E0342",
                    format!(
                        "the type parameter `{tp}` of `{mname}` shadows the type parameter `{tp}` \
                         of its `impl` block"
                    ),
                    "shadows an impl type parameter",
                    func.span,
                ));
                continue;
            }
            let fn_name = format!("{fn_prefix}::{mname}");
            if let Some(qualified) = &foreign {
                self.module_decls
                    .foreign_methods
                    .push((fn_name.clone(), qualified.clone()));
            }
            entries.push(MethodEntry {
                name: mname,
                receiver: method.receiver,
                fn_name: fn_name.clone(),
                is_pub: func.is_pub,
                module: String::new(),
                value: None,
                head: None,
            });

            let mut params = Vec::with_capacity(func.params.len() + 1);
            if method.receiver != Receiver::None {
                let mut ann = head.clone();
                ann.span = func.span;
                ann.reference = match method.receiver {
                    Receiver::Ref => Some(RefKind::Shared),
                    Receiver::RefMut => Some(RefKind::Mut),
                    _ if head.name == "str" => Some(RefKind::Shared),
                    _ => None,
                };
                params.push(Parameter::new(
                    "self".to_string(),
                    method.mut_self,
                    Some(ann),
                    func.span,
                ));
            }
            for mut p in std::mem::take(&mut func.params) {
                if let Some(ann) = p.type_annotation.as_mut() {
                    subst_ann(ann, &head);
                }
                params.push(p);
            }
            func.params = params;
            if let Some(ret) = func.return_type.as_mut() {
                subst_ann(ret, &head);
            }
            subst_stmts(&mut func.body, &head);

            let mut bounds: Vec<TypeBounds> = decl
                .type_params
                .iter()
                .map(|_| TypeBounds::default())
                .collect();
            for (i, b) in decl.bounds.iter().enumerate() {
                if let Some(slot) = bounds.get_mut(i) {
                    *slot = b.clone();
                }
            }
            let own = func.type_params.len();
            let mut own_bounds = std::mem::take(&mut func.bounds);
            own_bounds.resize(own, TypeBounds::default());
            bounds.extend(own_bounds);
            let mut type_params = decl.type_params.clone();
            type_params.extend(std::mem::take(&mut func.type_params));
            self.method_tparams
                .insert(fn_name.clone(), type_params.clone());
            func.type_params = type_params;
            func.bounds = bounds;
            func.name = fn_name;
            out.push(func);
        }
        out
    }

    pub(crate) fn lookup_method(&mut self, type_key: &str, name: &str, span: Span) -> MethodLookup {
        let mut hits: Vec<MethodEntry> = Vec::new();
        for table in [&self.methods, &self.trait_methods] {
            if let Some(entries) = table.get(type_key) {
                hits.extend(entries.iter().filter(|e| e.name == name).cloned());
            }
        }
        match hits.len() {
            0 => MethodLookup::Missing,
            1 => MethodLookup::Found(hits.pop().expect("one hit")),
            _ => {
                let names: Vec<String> = hits
                    .iter()
                    .map(|e| super::traits::provider_shown(&e.fn_name))
                    .collect();
                self.errors.push(TypeError::coded(
                    "E0759",
                    format!(
                        "`{name}` is ambiguous on `{}`: it is provided by both {}; call it by \
                         its trait path",
                        crate::modules::source_type_name(type_key),
                        names
                            .iter()
                            .map(|n| format!("`{n}`"))
                            .collect::<Vec<_>>()
                            .join(" and ")
                    ),
                    "ambiguous method",
                    span,
                ));
                MethodLookup::Ambiguous
            }
        }
    }

    pub(crate) fn method_names(&self, type_key: &str) -> Vec<String> {
        let mut names: Vec<String> = Vec::new();
        for table in [&self.methods, &self.trait_methods] {
            if let Some(entries) = table.get(type_key) {
                names.extend(
                    entries
                        .iter()
                        .filter(|e| e.receiver != Receiver::None)
                        .map(|e| e.name.clone()),
                );
            }
        }
        names.sort();
        names.dedup();
        names
    }
}

fn collect_names(ann: &TypeAnnotation, out: &mut HashSet<String>) {
    out.insert(ann.name.clone());
    if let Some(p) = &ann.type_param {
        collect_names(p, out);
    }
    for p in &ann.type_params {
        collect_names(p, out);
    }
    if let Some(ps) = &ann.fn_params {
        for p in ps {
            collect_names(p, out);
        }
    }
    if let Some(r) = &ann.fn_ret {
        collect_names(r, out);
    }
}

pub(crate) fn annotation_shown(ann: &TypeAnnotation) -> String {
    let mut s = String::new();
    match ann.reference {
        Some(RefKind::Mut) => s.push_str("&mut "),
        Some(RefKind::Shared) => s.push('&'),
        None => {}
    }
    s.push_str(crate::modules::source_type_name(&ann.name));
    let args: Vec<String> = if !ann.type_params.is_empty() {
        ann.type_params.iter().map(annotation_shown).collect()
    } else {
        ann.type_param.iter().map(|p| annotation_shown(p)).collect()
    };
    if !args.is_empty() {
        s.push('<');
        s.push_str(&args.join(", "));
        s.push('>');
    }
    s
}

pub(crate) fn subst_ann(ann: &mut TypeAnnotation, head: &TypeAnnotation) {
    if ann.name == "Self" && !ann.is_function_type() && !ann.is_slice {
        let reference = ann.reference;
        let span = ann.span;
        *ann = head.clone();
        ann.reference = reference;
        ann.span = span;
        return;
    }
    if let Some(p) = ann.type_param.as_mut() {
        subst_ann(p, head);
    }
    for p in ann.type_params.iter_mut() {
        subst_ann(p, head);
    }
    if let Some(ps) = ann.fn_params.as_mut() {
        for p in ps.iter_mut() {
            subst_ann(p, head);
        }
    }
    if let Some(r) = ann.fn_ret.as_mut() {
        subst_ann(r, head);
    }
}

fn subst_name(name: &mut String, head: &TypeAnnotation) {
    if name == "Self" {
        *name = head.name.clone();
    }
}

pub(crate) fn subst_stmts(stmts: &mut [Stmt], head: &TypeAnnotation) {
    for stmt in stmts {
        subst_stmt(stmt, head);
    }
}

fn subst_stmt(stmt: &mut Stmt, head: &TypeAnnotation) {
    match &mut stmt.kind {
        StmtKind::Expression(e) | StmtKind::Semi(e) | StmtKind::Discard(e) => subst_expr(e, head),
        StmtKind::Let {
            type_annotation,
            initializer,
            ..
        } => {
            if let Some(ann) = type_annotation.as_mut() {
                subst_ann(ann, head);
            }
            subst_expr(initializer, head);
        }
        StmtKind::LetPattern {
            pattern,
            type_annotation,
            initializer,
        } => {
            subst_pattern(pattern, head);
            if let Some(ann) = type_annotation.as_mut() {
                subst_ann(ann, head);
            }
            subst_expr(initializer, head);
        }
        StmtKind::Const {
            type_annotation,
            initializer,
            ..
        } => {
            subst_ann(type_annotation, head);
            subst_expr(initializer, head);
        }
        StmtKind::TypeAlias { target, .. } => subst_ann(target, head),
        StmtKind::Block(inner) => subst_stmts(inner, head),
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            subst_expr(condition, head);
            subst_stmt(then_branch, head);
            if let Some(e) = else_branch.as_mut() {
                subst_stmt(e, head);
            }
        }
        StmtKind::While { condition, body } => {
            subst_expr(condition, head);
            subst_stmt(body, head);
        }
        StmtKind::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            subst_expr(start, head);
            subst_expr(end, head);
            if let Some(s) = step.as_mut() {
                subst_expr(s, head);
            }
            subst_stmt(body, head);
        }
        StmtKind::ForEach { iterable, body, .. } => {
            subst_expr(iterable, head);
            subst_stmt(body, head);
        }
        StmtKind::Return(e) => {
            if let Some(e) = e.as_mut() {
                subst_expr(e, head);
            }
        }
        StmtKind::Function(func) => subst_function(func, head),
        StmtKind::Break(Some(e)) => subst_expr(e, head),
        StmtKind::Break(None)
        | StmtKind::Continue
        | StmtKind::Needs(_)
        | StmtKind::StructDecl { .. }
        | StmtKind::EnumDecl { .. }
        | StmtKind::Impl(_)
        | StmtKind::Trait(_) => {}
    }
}

fn subst_function(func: &mut Function, head: &TypeAnnotation) {
    for p in func.params.iter_mut() {
        if let Some(ann) = p.type_annotation.as_mut() {
            subst_ann(ann, head);
        }
    }
    if let Some(r) = func.return_type.as_mut() {
        subst_ann(r, head);
    }
    subst_stmts(&mut func.body, head);
}

fn subst_pattern(pattern: &mut Pattern, head: &TypeAnnotation) {
    match pattern {
        Pattern::Variant {
            enum_name, fields, ..
        } => {
            subst_name(enum_name, head);
            for p in fields {
                subst_pattern(p, head);
            }
        }
        Pattern::Tuple { elems, .. } | Pattern::Or(elems, _) => {
            for p in elems {
                subst_pattern(p, head);
            }
        }
        Pattern::Struct { name, fields, .. } => {
            subst_name(name, head);
            for (_, p, _) in fields {
                subst_pattern(p, head);
            }
        }
        Pattern::Binding { sub, .. } => {
            if let Some(p) = sub {
                subst_pattern(p, head);
            }
        }
        Pattern::ConstRange { lo, hi, .. } => {
            subst_pattern(lo, head);
            subst_pattern(hi, head);
        }
        Pattern::Wildcard(_) | Pattern::Lit(..) | Pattern::Range { .. } => {}
    }
}

fn subst_arms(arms: &mut [MatchArm], head: &TypeAnnotation) {
    for arm in arms {
        subst_pattern(&mut arm.pattern, head);
        if let Some(g) = arm.guard.as_mut() {
            subst_expr(g, head);
        }
        subst_expr(&mut arm.body, head);
    }
}

fn subst_expr(expr: &mut Expr, head: &TypeAnnotation) {
    match &mut expr.kind {
        ExprKind::Int(_)
        | ExprKind::Float(_)
        | ExprKind::String(_)
        | ExprKind::Char(_)
        | ExprKind::Bool(_)
        | ExprKind::Null
        | ExprKind::Panic(_)
        | ExprKind::Identifier(_) => {}
        ExprKind::FmtString(parts) => {
            for part in parts {
                if let FmtStringPart::Expr(e) = part {
                    subst_expr(e, head);
                }
            }
        }
        ExprKind::Binary { left, right, .. }
        | ExprKind::And { left, right }
        | ExprKind::Or { left, right } => {
            subst_expr(left, head);
            subst_expr(right, head);
        }
        ExprKind::Unary { operand, .. }
        | ExprKind::Grouping(operand)
        | ExprKind::Deref(operand)
        | ExprKind::Try(operand)
        | ExprKind::Unsafe(operand)
        | ExprKind::Reference { operand, .. } => subst_expr(operand, head),
        ExprKind::Call { callee, args } => {
            subst_expr(callee, head);
            for a in args {
                subst_expr(a, head);
            }
        }
        ExprKind::Assign { value, .. } => subst_expr(value, head),
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            subst_expr(condition, head);
            subst_expr(then_branch, head);
            subst_expr(else_branch, head);
        }
        ExprKind::Lambda {
            params,
            return_type,
            body,
        } => {
            for p in params.iter_mut() {
                if let Some(ann) = p.type_annotation.as_mut() {
                    subst_ann(ann, head);
                }
            }
            if let Some(r) = return_type.as_mut() {
                subst_ann(r, head);
            }
            subst_stmts(body, head);
        }
        ExprKind::Member { object, .. } => subst_expr(object, head),
        ExprKind::ArrayLiteral { elements } | ExprKind::Tuple(elements) => {
            for e in elements {
                subst_expr(e, head);
            }
        }
        ExprKind::ArraySized { size, fill_value } => {
            subst_expr(size, head);
            if let Some(f) = fill_value.as_mut() {
                subst_expr(f, head);
            }
        }
        ExprKind::VecLiteral {
            element_type,
            elements,
        } => {
            if let Some(ann) = element_type.as_mut() {
                subst_ann(ann, head);
            }
            for e in elements {
                subst_expr(e, head);
            }
        }
        ExprKind::Index { object, index } => {
            subst_expr(object, head);
            subst_expr(index, head);
        }
        ExprKind::IndexAssign {
            object,
            index,
            value,
        } => {
            subst_expr(object, head);
            subst_expr(index, head);
            subst_expr(value, head);
        }
        ExprKind::FieldAssign { object, value, .. } => {
            subst_expr(object, head);
            subst_expr(value, head);
        }
        ExprKind::Range { start, end, .. } => {
            if let Some(s) = start.as_mut() {
                subst_expr(s, head);
            }
            if let Some(e) = end.as_mut() {
                subst_expr(e, head);
            }
        }
        ExprKind::Slice { object, range } => {
            subst_expr(object, head);
            subst_expr(range, head);
        }
        ExprKind::DerefAssign { target, value } => {
            subst_expr(target, head);
            subst_expr(value, head);
        }
        ExprKind::StructLiteral { name, fields, base } => {
            subst_name(name, head);
            for f in fields {
                subst_expr(&mut f.value, head);
            }
            if let Some(base) = base {
                subst_expr(base, head);
            }
        }
        ExprKind::EnumVariant {
            enum_name, args, ..
        } => {
            subst_name(enum_name, head);
            for a in args {
                subst_expr(a, head);
            }
        }
        ExprKind::Cast {
            expr: inner,
            target,
        } => {
            subst_expr(inner, head);
            subst_ann(target, head);
        }
        ExprKind::Match {
            scrutinee, arms, ..
        } => {
            subst_expr(scrutinee, head);
            subst_arms(arms, head);
        }
        ExprKind::Block { stmts, tail } => {
            subst_stmts(stmts, head);
            subst_expr(tail, head);
        }
        ExprKind::Loop(body) => subst_stmts(body, head),
        ExprKind::Catch { scrutinee, handler } => {
            subst_expr(scrutinee, head);
            match handler {
                CatchHandler::Binding { body, .. } => subst_expr(body, head),
                CatchHandler::Arms(arms) => subst_arms(arms, head),
            }
        }
    }
}

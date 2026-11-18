mod consts;
mod walk;

use super::TypeInference;
use crate::constraint::{ConstraintReason, TypeError, TypeErrorKind};
use crate::modules::{AliasDef, ModuleExports, qualify_alias_body};
use crate::typed_ast::{ConstLit, ConstScalar};
use crate::types::InferType;
use aelys_syntax::{Span, Stmt, StmtKind, TypeAnnotation};
use consts::{ConstEval, ConstSource};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

struct LocalAlias {
    params: Vec<String>,
    target: TypeAnnotation,
    is_pub: bool,
    span: Span,
}

struct Aliases<'a> {
    local: &'a HashMap<String, LocalAlias>,
    resolved: HashMap<String, Option<TypeAnnotation>>,
    stack: Vec<String>,
    namespaces: &'a HashMap<String, Arc<ModuleExports>>,
    imported: &'a HashMap<String, AliasDef>,
    consts: &'a HashMap<String, Option<ConstLit>>,
    imported_consts: &'a HashMap<String, ConstLit>,
    errors: Vec<TypeError>,
}

fn arg_list(ann: &TypeAnnotation) -> Vec<TypeAnnotation> {
    if !ann.type_params.is_empty() {
        ann.type_params.clone()
    } else {
        ann.type_param.iter().map(|p| (**p).clone()).collect()
    }
}

fn substitute(body: &mut TypeAnnotation, params: &[String], args: &[TypeAnnotation]) {
    let bare = body.type_param.is_none()
        && body.type_params.is_empty()
        && !body.is_function_type()
        && body.tuple_elems.is_none();
    if bare && let Some(i) = params.iter().position(|p| *p == body.name) {
        let reference = body.reference;
        let span = body.span;
        *body = args[i].clone();
        if reference.is_some() {
            body.reference = reference;
        }
        body.span = span;
        return;
    }
    if let Some(p) = &mut body.type_param {
        substitute(p, params, args);
    }
    for p in &mut body.type_params {
        substitute(p, params, args);
    }
    if let Some(ps) = &mut body.fn_params {
        for p in ps {
            substitute(p, params, args);
        }
    }
    if let Some(r) = &mut body.fn_ret {
        substitute(r, params, args);
    }
    if let Some(es) = &mut body.tuple_elems {
        for e in es {
            substitute(e, params, args);
        }
    }
}

impl Aliases<'_> {
    fn array_size(&mut self, ann: &mut TypeAnnotation) {
        if ann.array_size.is_some() {
            return;
        }
        let Some(name) = ann.array_size_name.clone() else {
            return;
        };
        let lit = match name.split_once('.') {
            Some((ns, member)) => self
                .namespaces
                .get(ns)
                .and_then(|ex| ex.values.get(member))
                .filter(|item| item.is_pub)
                .and_then(|item| self.imported_consts.get(&item.qualified))
                .cloned(),
            None => match self.consts.get(&name) {
                Some(found) => found.clone(),
                None => self.imported_consts.get(&name).cloned(),
            },
        };
        let Some(lit) = lit else {
            if self.consts.get(&name).is_some_and(|c| c.is_none()) {
                return;
            }
            self.errors.push(TypeError::coded(
                "E0301",
                format!("the array size `{name}` is not a const"),
                "not a const",
                ann.span,
            ));
            ann.array_size = Some(0);
            return;
        };
        match lit.value {
            ConstScalar::Int(v) if lit.ty.is_integer() && v >= 0 => {
                ann.array_size = Some(v as u64);
            }
            _ => {
                self.errors.push(TypeError::coded(
                    "E0301",
                    format!(
                        "the array size `{name}` must be a non-negative integer const, \
                         found a `{}`",
                        lit.ty
                    ),
                    "not a length",
                    ann.span,
                ));
                ann.array_size = Some(0);
            }
        }
    }

    fn lookup(&mut self, name: &str, span: Span) -> Option<(Vec<String>, TypeAnnotation)> {
        if self.local.contains_key(name) {
            let params = self.local[name].params.clone();
            let body = self.resolve(name, span)?;
            return Some((params, body));
        }
        if let Some((ns, member)) = name.split_once('.')
            && let Some(ex) = self.namespaces.get(ns)
            && let Some(def) = ex.aliases.get(member)
        {
            if !def.is_pub {
                self.errors.push(TypeError {
                    kind: TypeErrorKind::ModuleItemNotPublic {
                        module: ex.path.clone(),
                        item: member.to_string(),
                    },
                    span,
                    reason: ConstraintReason::Other(String::new()),
                    secondary_spans: Vec::new(),
                    help: None,
                    suggestion: None,
                });
                return None;
            }
            return Some((def.params.clone(), def.target.clone()));
        }
        let def = self.imported.get(name)?;
        Some((def.params.clone(), def.target.clone()))
    }

    fn is_alias(&self, name: &str) -> bool {
        self.local.contains_key(name)
            || self.imported.contains_key(name)
            || name.split_once('.').is_some_and(|(ns, member)| {
                self.namespaces
                    .get(ns)
                    .is_some_and(|ex| ex.aliases.contains_key(member))
            })
    }

    fn resolve(&mut self, name: &str, use_span: Span) -> Option<TypeAnnotation> {
        if let Some(done) = self.resolved.get(name) {
            return done.clone();
        }
        if let Some(start) = self.stack.iter().position(|n| n == name) {
            let mut chain = self.stack[start..].to_vec();
            chain.push(name.to_string());
            let head = chain[0].clone();
            let span = self.local.get(&head).map(|a| a.span).unwrap_or(use_span);
            self.errors.push(TypeError::coded(
                "E0305",
                format!(
                    "the type alias `{head}` refers to itself: {}",
                    chain.join(" -> ")
                ),
                "recursive type alias",
                span,
            ));
            for n in &chain {
                self.resolved.insert(n.clone(), None);
            }
            return None;
        }
        self.stack.push(name.to_string());
        let mut body = self.local[name].target.clone();
        let ok = self.expand(&mut body);
        self.stack.pop();
        let result = if ok && !self.resolved.contains_key(name) {
            Some(body)
        } else {
            None
        };
        self.resolved.insert(name.to_string(), result.clone());
        result
    }

    fn expand(&mut self, ann: &mut TypeAnnotation) -> bool {
        let mut ok = true;
        if let Some(p) = &mut ann.type_param {
            ok &= self.expand(p);
        }
        for p in &mut ann.type_params {
            ok &= self.expand(p);
        }
        if let Some(ps) = &mut ann.fn_params {
            for p in ps {
                ok &= self.expand(p);
            }
        }
        if let Some(r) = &mut ann.fn_ret {
            ok &= self.expand(r);
        }
        if let Some(es) = &mut ann.tuple_elems {
            for e in es {
                ok &= self.expand(e);
            }
        }
        self.array_size(ann);
        if ann.is_function_type() || ann.tuple_elems.is_some() || !self.is_alias(&ann.name) {
            return ok;
        }
        let name = ann.name.clone();
        let Some((params, body)) = self.lookup(&name, ann.span) else {
            return false;
        };
        let args = arg_list(ann);
        if args.len() != params.len() {
            self.errors.push(TypeError::type_arg_count(
                "type alias",
                &name,
                params.len(),
                args.len(),
                ann.span,
            ));
            return false;
        }
        let mut out = body;
        substitute(&mut out, &params, &args);
        if ann.reference.is_some() {
            out.reference = ann.reference;
        }
        out.span = ann.span;
        *ann = out;
        ok
    }
}

impl TypeInference {
    pub(super) fn prepare_decls(&mut self, stmts: &mut Vec<Stmt>) {
        let mut local: HashMap<String, LocalAlias> = HashMap::new();
        let mut alias_order: Vec<String> = Vec::new();
        let mut sources: HashMap<String, ConstSource> = HashMap::new();
        let mut const_order: Vec<(String, bool)> = Vec::new();
        let mut owned: HashSet<String> = HashSet::new();
        stmts.retain(|s| match &s.kind {
            StmtKind::Const {
                name,
                type_annotation,
                initializer,
                is_pub,
            } => {
                sources.insert(
                    name.clone(),
                    ConstSource {
                        ann: type_annotation.clone(),
                        init: initializer.clone(),
                    },
                );
                const_order.push((name.clone(), *is_pub));
                false
            }
            StmtKind::TypeAlias {
                name,
                type_params,
                target,
                is_pub,
            } => {
                local.insert(
                    name.clone(),
                    LocalAlias {
                        params: type_params.clone(),
                        target: target.clone(),
                        is_pub: *is_pub,
                        span: s.span,
                    },
                );
                alias_order.push(name.clone());
                false
            }
            StmtKind::StructDecl { name, .. } | StmtKind::EnumDecl { name, .. } => {
                owned.insert(name.clone());
                true
            }
            _ => true,
        });
        self.alias_names = local.keys().cloned().collect();

        let namespaces = self.module_imports.namespaces.clone();
        let imported_aliases = self.module_imports.aliases.clone();
        let imported_consts = self.consts.clone();
        let empty_consts: HashMap<String, Option<ConstLit>> = HashMap::new();

        let mut const_types: HashMap<String, TypeAnnotation> = HashMap::new();
        {
            let mut aliases = Aliases {
                local: &local,
                resolved: HashMap::new(),
                stack: Vec::new(),
                namespaces: &namespaces,
                imported: &imported_aliases,
                consts: &empty_consts,
                imported_consts: &imported_consts,
                errors: Vec::new(),
            };
            for (name, src) in &sources {
                let mut ann = src.ann.clone();
                aliases.expand(&mut ann);
                const_types.insert(name.clone(), ann);
            }
            self.errors.extend(aliases.errors);
        }
        for (name, ann) in const_types {
            if let Some(src) = sources.get_mut(&name) {
                src.ann = ann;
            }
        }

        let mut evaluated: HashMap<String, Option<ConstLit>> = HashMap::new();
        {
            let mut eval = ConstEval::new(&sources, &imported_consts, &namespaces);
            for (name, _) in &const_order {
                let lit = eval.eval_const(name);
                evaluated.insert(name.clone(), lit);
            }
            self.errors.extend(eval.errors);
        }

        let mut aliases = Aliases {
            local: &local,
            resolved: HashMap::new(),
            stack: Vec::new(),
            namespaces: &namespaces,
            imported: &imported_aliases,
            consts: &evaluated,
            imported_consts: &imported_consts,
            errors: Vec::new(),
        };
        for name in &alias_order {
            aliases.resolve(name, local[name].span);
        }
        walk::stmts(stmts, &mut |ann| {
            aliases.expand(ann);
        });
        let resolved = std::mem::take(&mut aliases.resolved);
        self.errors.extend(aliases.errors);

        for name in &alias_order {
            let Some(Some(body)) = resolved.get(name) else {
                continue;
            };
            let def = &local[name];
            let mut target = body.clone();
            qualify_alias_body(&mut target, "", &owned, &def.params, Some(&namespaces));
            self.module_decls.aliases.push((
                name.clone(),
                AliasDef {
                    params: def.params.clone(),
                    target,
                    is_pub: def.is_pub,
                },
            ));
        }
        for (name, is_pub) in const_order {
            if let Some(Some(lit)) = evaluated.get(&name) {
                self.consts.insert(name.clone(), lit.clone());
                self.module_decls.consts.push((name, is_pub, lit.clone()));
            }
        }
    }

    pub(crate) fn const_literal(
        &self,
        name: &str,
        span: Span,
    ) -> Option<(crate::typed_ast::TypedExprKind, InferType)> {
        use crate::typed_ast::{TypedExpr, TypedExprKind};
        if self.env.lookup(name).is_some() || self.env.lookup_function_ref(name).is_some() {
            return None;
        }
        let lit = self.consts.get(name)?;
        let leaf = |kind: TypedExprKind| TypedExpr {
            kind,
            ty: lit.ty.clone(),
            span,
        };
        let kind = match &lit.value {
            ConstScalar::Int(v) => TypedExprKind::Cast {
                expr: Box::new(leaf(TypedExprKind::Int(*v))),
                target: lit.ty.clone(),
            },
            ConstScalar::Float(v) => TypedExprKind::Cast {
                expr: Box::new(leaf(TypedExprKind::Float(*v))),
                target: lit.ty.clone(),
            },
            ConstScalar::Bool(b) => TypedExprKind::Bool(*b),
            ConstScalar::Char(c) => TypedExprKind::Char(*c),
            ConstScalar::Str(s) => TypedExprKind::String(s.clone()),
        };
        Some((kind, lit.ty.clone()))
    }
}

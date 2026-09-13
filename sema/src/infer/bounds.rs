use super::TypeInference;
use crate::constraint::TypeError;
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Function, Span, TypeAnnotation, TypeBounds};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Bound {
    Nogc,
    Eq,
    Ord,
}

impl Bound {
    fn required_by(set: &TypeBounds) -> Vec<Bound> {
        let mut wanted = Vec::new();
        if set.nogc {
            wanted.push(Bound::Nogc);
        }
        if set.needs_ord() {
            wanted.push(Bound::Ord);
        } else if set.needs_eq() {
            wanted.push(Bound::Eq);
        }
        wanted
    }

    fn article(self) -> &'static str {
        match self {
            Bound::Nogc => "a",
            Bound::Eq | Bound::Ord => "an",
        }
    }

    fn spelling(self) -> &'static str {
        match self {
            Bound::Nogc => "nogc",
            Bound::Eq => "eq",
            Bound::Ord => "ord",
        }
    }
}

/// it lives here and never on `infertype::function`, which derives `partialeq/eq/hash` and is
pub(crate) struct BoundGenericSig {
    pub type_params: Vec<String>,
    pub bound: Vec<TypeBounds>,
    pub slots: Vec<Vec<usize>>,
}

fn type_mentions(ty: &InferType, type_param: &str) -> bool {
    match ty {
        InferType::Struct(name) => name == type_param,
        InferType::Array(inner, _) | InferType::Vec(inner) | InferType::Rc(inner) => {
            type_mentions(inner, type_param)
        }
        InferType::Ref { referent, .. } => type_mentions(referent, type_param),
        InferType::Slice { elem, .. } => type_mentions(elem, type_param),
        InferType::Tuple(elems) => elems.iter().any(|e| type_mentions(e, type_param)),
        InferType::Enum(_, args) => args.iter().any(|a| type_mentions(a, type_param)),
        InferType::Function { params, ret, .. } => {
            params.iter().any(|p| type_mentions(p, type_param)) || type_mentions(ret, type_param)
        }
        _ => false,
    }
}

fn annotation_mentions(ann: &TypeAnnotation, type_param: &str) -> bool {
    if ann.name == type_param {
        return true;
    }
    ann.type_param
        .iter()
        .any(|t| annotation_mentions(t, type_param))
        || ann
            .type_params
            .iter()
            .any(|t| annotation_mentions(t, type_param))
        || ann
            .fn_params
            .iter()
            .flatten()
            .any(|t| annotation_mentions(t, type_param))
        || ann
            .fn_ret
            .iter()
            .any(|t| annotation_mentions(t, type_param))
}

impl TypeInference {
    /// a name is a list of candidates, never a single entry: the same bare name is registered for
    pub(super) fn record_bound_generic_sig(&mut self, key: &str, func: &Function) {
        if func.type_params.is_empty() {
            return;
        }
        let bound: Vec<TypeBounds> = (0..func.type_params.len())
            .map(|i| {
                let mut set = func.bounds.get(i).copied().unwrap_or_default();
                set.nogc |= func.is_nogc;
                set
            })
            .collect();
        if bound.iter().all(TypeBounds::is_empty) {
            return;
        }
        let slots: Vec<Vec<usize>> = func
            .type_params
            .iter()
            .map(|tp| {
                func.params
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| {
                        p.type_annotation
                            .as_ref()
                            .is_some_and(|ann| annotation_mentions(ann, tp))
                    })
                    .map(|(i, _)| i)
                    .collect()
            })
            .collect();
        let candidates = self.bound_generic_sigs.entry(key.to_string()).or_default();
        if candidates
            .iter()
            .any(|c| c.type_params == func.type_params && c.bound == bound && c.slots == slots)
        {
            return;
        }
        candidates.push(BoundGenericSig {
            type_params: func.type_params.clone(),
            bound,
            slots,
        });
    }

    pub(crate) fn record_imported_bound_sig(&mut self, item: &crate::modules::ModuleValue) {
        if item.type_params.is_empty() || item.bounds.iter().all(TypeBounds::is_empty) {
            return;
        }
        let InferType::Function { params, .. } = &item.ty else {
            return;
        };
        let slots: Vec<Vec<usize>> = item
            .type_params
            .iter()
            .map(|tp| {
                params
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| type_mentions(p, tp))
                    .map(|(i, _)| i)
                    .collect()
            })
            .collect();
        let candidates = self
            .bound_generic_sigs
            .entry(item.qualified.clone())
            .or_default();
        if candidates.iter().any(|c| {
            c.type_params == item.type_params && c.bound == item.bounds && c.slots == slots
        }) {
            return;
        }
        candidates.push(BoundGenericSig {
            type_params: item.type_params.clone(),
            bound: item.bounds.clone(),
            slots,
        });
    }

    /// without this a bounded generic could not hand its own parameter to another one
    fn binding_satisfies(&self, ty: &InferType, want: Bound) -> bool {
        if let Some((_, carried)) = self.bounds_on_type_param(ty) {
            return match want {
                Bound::Nogc => carried.nogc,
                Bound::Eq => carried.needs_eq(),
                Bound::Ord => carried.needs_ord(),
            };
        }
        match want {
            Bound::Nogc => self.type_table.is_nogc_value(ty),
            Bound::Eq => ty.satisfies_eq(),
            Bound::Ord => ty.satisfies_ord(),
        }
    }

    /// concrete value satisfying it. fail-closed: a binding the matcher cannot recover is a reject.
    pub(super) fn check_bound_call(&mut self, callee: &TypedExpr, args: &[TypedExpr]) {
        // any other callee form reaches the name through a value use, already rejected as one
        let TypedExprKind::Identifier(name) = &callee.kind else {
            return;
        };
        let Some(sigs) = self.bound_generic_sigs.get(name.as_str()) else {
            return;
        };
        let candidates: Vec<(Vec<String>, Vec<TypeBounds>, Vec<Vec<usize>>)> = sigs
            .iter()
            .map(|sig| {
                (
                    sig.type_params.clone(),
                    sig.bound.clone(),
                    sig.slots.clone(),
                )
            })
            .collect();
        let InferType::Function { params, .. } = &callee.ty else {
            return;
        };
        let sig_params = params.clone();

        let mut reported: HashSet<String> = HashSet::new();
        for (type_params, bound, slots) in candidates {
            let mut bindings: HashMap<String, (InferType, Span)> = HashMap::new();
            for (param_ty, arg) in sig_params.iter().zip(args.iter()) {
                self.bind_type_params(param_ty, &arg.ty, arg.span, &type_params, &mut bindings);
            }

            for (i, type_param) in type_params.iter().enumerate() {
                let set = bound[i];
                if set.is_empty() || reported.contains(type_param) {
                    continue;
                }
                let param_slots: &[usize] = slots.get(i).map_or(&[], |s| s);
                let mut err = None;
                for want in Bound::required_by(&set) {
                    if let Some((ty, _)) = bindings.get(type_param)
                        && self.binding_satisfies(ty, want)
                    {
                        continue;
                    }
                    err = Some(self.bound_failure(
                        name,
                        type_param,
                        want,
                        bindings.get(type_param),
                        param_slots,
                        args,
                        callee.span,
                    ));
                    break;
                }
                if let Some(err) = err {
                    reported.insert(type_param.clone());
                    self.errors.push(err);
                }
            }
        }
    }

    fn bound_failure(
        &self,
        name: &str,
        type_param: &str,
        want: Bound,
        binding: Option<&(InferType, Span)>,
        param_slots: &[usize],
        args: &[TypedExpr],
        callee_span: Span,
    ) -> TypeError {
        let carries = || {
            format!(
                "`{type_param}` of `{name}` carries {} `{}` bound",
                want.article(),
                want.spelling()
            )
        };
        if want != Bound::Nogc {
            return match binding {
                Some((ty, arg_span)) => {
                    TypeError::bound_not_satisfied(name, type_param, want.spelling(), ty, *arg_span)
                        .with_secondary(callee_span, carries())
                }
                None => TypeError::bound_unresolved(name, type_param, want.spelling(), callee_span),
            };
        }
        match binding {
            Some((ty, arg_span)) => {
                TypeError::nogc_bound_violation(name, type_param, ty, *arg_span)
                    .with_secondary(callee_span, carries())
            }
            None => match self.generic_struct_culprit(param_slots, args) {
                Some((struct_name, arg_span)) => TypeError::nogc_bound_generic_struct_arg(
                    name,
                    type_param,
                    &struct_name,
                    arg_span,
                )
                .with_secondary(callee_span, carries()),
                None => TypeError::nogc_bound_unresolved(name, type_param, callee_span),
            },
        }
    }

    fn generic_struct_culprit(
        &self,
        slots: &[usize],
        args: &[TypedExpr],
    ) -> Option<(String, Span)> {
        let mut culprit = None;
        for &slot in slots {
            let Some(arg) = args.get(slot) else { continue };
            let InferType::Struct(struct_name) = &arg.ty else {
                continue;
            };
            let is_generic = self
                .type_table
                .get_struct(struct_name)
                .is_some_and(|def| !def.type_params.is_empty());
            if !is_generic {
                continue;
            }
            if culprit.is_some() {
                return None;
            }
            culprit = Some((struct_name.clone(), arg.span));
        }
        culprit
    }

    /// argument type in lockstep. `unify` cannot do this (it binds only `var` and matches `struct`
    fn bind_type_params(
        &self,
        sig: &InferType,
        arg: &InferType,
        arg_span: Span,
        type_params: &[String],
        out: &mut HashMap<String, (InferType, Span)>,
    ) {
        if let InferType::Struct(name) = sig
            && type_params.iter().any(|p| p == name)
            && !self.type_table.has_struct(name)
        {
            out.entry(name.clone())
                .or_insert_with(|| (arg.clone(), arg_span));
            return;
        }
        match (sig, arg) {
            (InferType::Array(si, _), InferType::Array(ai, _))
            | (InferType::Vec(si), InferType::Vec(ai))
            | (InferType::Rc(si), InferType::Rc(ai)) => {
                self.bind_type_params(si, ai, arg_span, type_params, out)
            }
            (InferType::Ref { referent: sr, .. }, InferType::Ref { referent: ar, .. }) => {
                self.bind_type_params(sr, ar, arg_span, type_params, out)
            }
            (InferType::Slice { elem: se, .. }, InferType::Slice { elem: ae, .. }) => {
                self.bind_type_params(se, ae, arg_span, type_params, out)
            }
            (InferType::Tuple(se), InferType::Tuple(ae)) if se.len() == ae.len() => {
                for (s, a) in se.iter().zip(ae.iter()) {
                    self.bind_type_params(s, a, arg_span, type_params, out);
                }
            }
            (InferType::Enum(sn, sa), InferType::Enum(an, aa))
                if sn == an && sa.len() == aa.len() =>
            {
                for (s, a) in sa.iter().zip(aa.iter()) {
                    self.bind_type_params(s, a, arg_span, type_params, out);
                }
            }
            (
                InferType::Function {
                    params: sp,
                    ret: sr,
                    ..
                },
                InferType::Function {
                    params: ap,
                    ret: ar,
                    ..
                },
            ) if sp.len() == ap.len() => {
                for (s, a) in sp.iter().zip(ap.iter()) {
                    self.bind_type_params(s, a, arg_span, type_params, out);
                }
                self.bind_type_params(sr, ar, arg_span, type_params, out);
            }
            _ => {}
        }
    }

    pub(super) fn record_foreign_sig(&mut self, key: &str, func: &Function) {
        if func.foreign.is_some() {
            self.foreign_sigs.insert(key.to_string());
        }
    }

    pub(super) fn check_nogc_generic_value(&mut self, name: &str, expr: &TypedExpr) {
        if !matches!(expr.ty, InferType::Function { .. }) {
            return;
        }
        if !self
            .bound_generic_sigs
            .get(name)
            .is_some_and(|sigs| sigs.iter().any(|s| s.bound.iter().any(|b| b.nogc)))
        {
            return;
        }
        let err = TypeError::nogc_generic_as_value(name, expr.span);
        self.errors.push(err);
    }

    pub(super) fn foreign_sigs_shadowed_by_local(&self, name: &str) -> bool {
        self.foreign_sigs.contains(name) && self.env.lookup_local(name).is_some()
    }

    pub(super) fn check_foreign_as_value(&mut self, name: &str, expr: &TypedExpr) {
        if !matches!(expr.ty, InferType::Function { .. }) {
            return;
        }
        if !self.foreign_sigs.contains(name) {
            return;
        }
        if self
            .foreign_shadowed_spans
            .contains(&(expr.span.start, expr.span.end))
        {
            return;
        }
        let err = TypeError::foreign_as_value(name, expr.span);
        self.errors.push(err);
    }
}

use super::TypeInference;
use crate::constraint::TypeError;
use crate::typed_ast::{TypedExpr, TypedExprKind};
use crate::types::InferType;
use aelys_syntax::{Function, Span, TypeAnnotation};
use std::collections::{HashMap, HashSet};

/// it lives here and never on `infertype::function`, which derives `partialeq/eq/hash` and is
pub(crate) struct NogcGenericSig {
    pub type_params: Vec<String>,
    pub bound: Vec<bool>,
    pub slots: Vec<Vec<usize>>,
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
    /// binds only what it wrote as `<t: nogc>`. nothing is recorded when nothing is bound, so the
    /// a name is a list of candidates, never a single entry: the same bare name is registered for
    /// every scope (`signatures.rs`), and the post-solve check has no scope state, so an entry that
    /// could be overwritten would let a weaker same-named signature erase a bound. recording only
    /// ever appends, and a call must satisfy every candidate, so an insert cannot weaken the table.
    pub(super) fn record_nogc_generic_sig(&mut self, key: &str, func: &Function) {
        if func.type_params.is_empty() {
            return;
        }
        let bound: Vec<bool> = (0..func.type_params.len())
            .map(|i| func.is_nogc || func.nogc_bounds.get(i).copied().unwrap_or(false))
            .collect();
        if !bound.iter().any(|b| *b) {
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
        let candidates = self.nogc_generic_sigs.entry(key.to_string()).or_default();
        if candidates
            .iter()
            .any(|c| c.type_params == func.type_params && c.bound == bound && c.slots == slots)
        {
            return;
        }
        candidates.push(NogcGenericSig {
            type_params: func.type_params.clone(),
            bound,
            slots,
        });
    }

    /// at a call instantiating a nogc-bound generic, every bound type param must resolve to a
    /// concrete nogc value. fail-closed: a binding the matcher cannot recover is a reject.
    pub(super) fn check_nogc_bound_call(&mut self, callee: &TypedExpr, args: &[TypedExpr]) {
        // any other callee form reaches the name through a value use, already rejected as one
        let TypedExprKind::Identifier(name) = &callee.kind else {
            return;
        };
        let Some(sigs) = self.nogc_generic_sigs.get(name.as_str()) else {
            return;
        };
        let candidates: Vec<(Vec<String>, Vec<bool>, Vec<Vec<usize>>)> = sigs
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
                if !bound[i] {
                    continue;
                }
                let carries_bound = || format!("`{type_param}` of `{name}` carries a `nogc` bound");
                let param_slots: &[usize] = slots.get(i).map_or(&[], |s| s);
                let err = match bindings.get(type_param) {
                    Some((ty, _)) if self.type_table.is_nogc_value(ty) => continue,
                    Some((ty, arg_span)) => {
                        TypeError::nogc_bound_violation(name, type_param, ty, *arg_span)
                            .with_secondary(callee.span, carries_bound())
                    }
                    None => match self.generic_struct_culprit(param_slots, args) {
                        Some((struct_name, arg_span)) => TypeError::nogc_bound_generic_struct_arg(
                            name,
                            type_param,
                            &struct_name,
                            arg_span,
                        )
                        .with_secondary(callee.span, carries_bound()),
                        // nothing pins the param, so the call itself is the only place to point at
                        None => TypeError::nogc_bound_unresolved(name, type_param, callee.span),
                    },
                };
                if reported.insert(type_param.clone()) {
                    self.errors.push(err);
                }
            }
        }
    }

    /// only a slot the declaration ties to the param qualifies, and only when there is exactly one,
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

    /// the bound, so only a direct call is allowed.
    pub(super) fn check_nogc_generic_value(&mut self, name: &str, expr: &TypedExpr) {
        if !matches!(expr.ty, InferType::Function { .. }) {
            return;
        }
        if !self.nogc_generic_sigs.contains_key(name) {
            return;
        }
        let err = TypeError::nogc_generic_as_value(name, expr.span);
        self.errors.push(err);
    }
}

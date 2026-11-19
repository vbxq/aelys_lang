use crate::bir::effects::{Effect, EffectSet};
use crate::passes::instance_surface::InstanceSurfaceError;
use crate::{AirFunction, AirProgram, AirStmtKind, AirTerminator, Callee, Rvalue, Span};
use std::collections::{HashMap, HashSet};

fn callees(function: &AirFunction) -> Vec<(String, Option<Span>)> {
    let mut out = Vec::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                AirStmtKind::Assign {
                    rvalue:
                        Rvalue::Call {
                            func: Callee::Named(n),
                            ..
                        },
                    ..
                }
                | AirStmtKind::CallVoid {
                    func: Callee::Named(n),
                    ..
                } => out.push((n.clone(), stmt.span)),
                _ => {}
            }
        }
        if let AirTerminator::Invoke {
            func: Callee::Named(n),
            ..
        } = &block.terminator
        {
            out.push((n.clone(), None));
        }
    }
    out
}

fn source_name(name: &str) -> String {
    let bare = name.strip_prefix("__q.").unwrap_or(name);
    bare.rsplit('.').next().unwrap_or(bare).to_string()
}

pub fn shown_symbol(name: &str, origin: &HashMap<String, String>) -> String {
    let name = origin.get(name).map_or(name, |o| o.as_str());
    if let Some(rest) = name.strip_prefix("__impl$") {
        let q = rest.split('$').next().unwrap_or(rest);
        let m = rest.rsplit('$').next().unwrap_or(rest);
        let ty = rest
            .strip_prefix(q)
            .and_then(|r| r.strip_prefix('$'))
            .and_then(|r| r.strip_suffix(m))
            .and_then(|r| r.strip_suffix('$'))
            .unwrap_or(rest);
        return format!("<{} as {}>::{m}", source_name(ty), source_name(q));
    }
    if let Some(rest) = name.strip_prefix("__trait$") {
        let q = rest.split('$').next().unwrap_or(rest);
        let m = rest.rsplit('$').next().unwrap_or(rest);
        return format!("{}::{m}", source_name(q));
    }
    source_name(name)
}

pub fn check_instance_effects(
    program: &mut AirProgram,
    bir: &HashMap<String, EffectSet>,
    declared_nogc: &HashSet<String>,
) -> Vec<InstanceSurfaceError> {
    let ids: HashMap<_, _> = program
        .functions
        .iter()
        .map(|f| (f.id, f.name.clone()))
        .collect();
    let origin: HashMap<String, String> = program
        .mono_instances
        .iter()
        .filter_map(|i| Some((ids.get(&i.result)?.clone(), i.original_name.clone())))
        .collect();
    let names: HashSet<String> = program
        .functions
        .iter()
        .filter(|f| !f.is_extern)
        .map(|f| f.name.clone())
        .collect();
    let base = |name: &str| {
        let key = origin.get(name).map_or(name, |o| o.as_str());
        bir.get(key)
            .is_some_and(|e| e.contains(Effect::Managed))
    };
    let edges: HashMap<String, Vec<(String, Option<Span>)>> = program
        .functions
        .iter()
        .filter(|f| !f.is_extern)
        .map(|f| {
            let cs = callees(f)
                .into_iter()
                .filter(|(n, _)| names.contains(n) && n != &f.name)
                .collect();
            (f.name.clone(), cs)
        })
        .collect();
    let mut managed: HashSet<String> = names.iter().filter(|n| base(n)).cloned().collect();
    let mut changed = true;
    while changed {
        changed = false;
        for (name, cs) in &edges {
            if managed.contains(name) {
                continue;
            }
            if cs.iter().any(|(c, _)| managed.contains(c)) {
                managed.insert(name.clone());
                changed = true;
            }
        }
    }
    let is_declared = |name: &str| {
        declared_nogc.contains(name)
            || origin.get(name).is_some_and(|o| declared_nogc.contains(o))
    };
    let instance_names: HashSet<&String> = origin.keys().collect();
    let mut errors = Vec::new();
    let mut reported: HashSet<String> = HashSet::new();
    for function in &program.functions {
        if !managed.contains(&function.name) || !is_declared(&function.name) {
            continue;
        }
        let mut chain = vec![function.name.clone()];
        let mut cur = function.name.clone();
        let mut site = None;
        for _ in 0..names.len() {
            if base(&cur) && cur != function.name {
                break;
            }
            let Some((next, span)) = edges
                .get(&cur)
                .and_then(|cs| cs.iter().find(|(c, _)| managed.contains(c) && !chain.contains(c)))
                .cloned()
            else {
                break;
            };
            if site.is_none() {
                site = span;
            }
            chain.push(next.clone());
            cur = next;
        }
        let Some(witness) = chain
            .iter()
            .find(|n| {
                n.starts_with("__impl$")
                    || origin.get(*n).is_some_and(|o| o.starts_with("__impl$") || o.starts_with("__trait$"))
            })
            .cloned()
        else {
            continue;
        };
        if !reported.insert(function.name.clone()) {
            continue;
        }
        let shown: Vec<String> = chain.iter().map(|n| shown_symbol(n, &origin)).collect();
        let mut span = site.or(function.span);
        if instance_names.contains(&function.name)
            && let Some(s) = program.functions.iter().find_map(|f| {
                if instance_names.contains(&f.name) {
                    return None;
                }
                edges
                    .get(&f.name)?
                    .iter()
                    .find(|(c, _)| *c == function.name)
                    .map(|(_, s)| s.or(f.span))
            })
        {
            span = s;
        }
        errors.push(InstanceSurfaceError {
            code: "E0727",
            message: format!(
                "`{}` is `nogc`, and with these type arguments it calls `{}`, which may reach \
                 managed memory: {}",
                shown_symbol(&function.name, &origin),
                shown_symbol(&witness, &origin),
                shown.join(" -> ")
            ),
            span,
        });
    }
    for function in &mut program.functions {
        if managed.contains(&function.name) {
            function.attributes.no_gc = false;
        }
    }
    errors
}

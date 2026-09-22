use std::collections::HashMap;

use aelys_syntax::Span;

use super::*;

pub struct BirCheck {
    pub errors: Vec<BirDiagnostic>,
    pub drops: HashMap<DropKey, Vec<DropKey>>,
    pub effects: HashMap<String, super::EffectSet>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Lat {
    Uninit,
    Live,
    Moved,
    Maybe,
}

fn join(a: Lat, b: Lat) -> Lat {
    if a == b { a } else { Lat::Maybe }
}

fn join_state(a: &[Lat], b: &[Lat]) -> Vec<Lat> {
    a.iter().zip(b.iter()).map(|(x, y)| join(*x, *y)).collect()
}

pub fn check_program(bir: &BirProgram) -> BirCheck {
    check_program_with_imports(bir, &HashMap::new())
}

pub fn check_program_with_imports(
    bir: &BirProgram,
    imported: &HashMap<String, super::EffectSet>,
) -> BirCheck {
    check_program_with_chains(bir, imported, &HashMap::new())
}

pub fn check_program_with_chains(
    bir: &BirProgram,
    imported: &HashMap<String, super::EffectSet>,
    chains: &HashMap<String, Vec<super::Step>>,
) -> BirCheck {
    let mut errors: Vec<BirDiagnostic> = Vec::new();
    let mut drops: HashMap<DropKey, Vec<DropKey>> = HashMap::new();
    for body in &bir.bodies {
        check_body(body, &mut errors, &mut drops);
    }
    // origin summaries + the d1/floor return-escape diagnostics
    let summaries = super::origins::summaries(bir, &mut errors);
    errors.extend(super::loans::check(bir, &summaries));
    for body in &bir.bodies {
        errors.extend(body.build_errors.iter().cloned());
    }
    let effects = super::effects::effect_summaries_with_imports(bir, imported);
    for body in &bir.bodies {
        if body.declared_nogc && !effects.get(&body.name).is_some_and(|e| e.is_nogc()) {
            errors.push(nogc_diagnostic(bir, &effects, chains, body));
        }
    }
    BirCheck {
        errors,
        drops,
        effects,
    }
}

const CHAIN_HEAD: usize = 3;
const CHAIN_TAIL: usize = 3;
const MAX_SECONDARY: usize = 4;

fn nogc_diagnostic(
    bir: &BirProgram,
    effects: &HashMap<String, EffectSet>,
    chains: &HashMap<String, Vec<super::Step>>,
    body: &BirBody,
) -> BirDiagnostic {
    use super::effects::StepKind;

    let mut message = format!(
        "[nogc] function `{}` is declared nogc but its inferred effects reach managed memory",
        body.name
    );
    let chain = super::effects::managed_chain_with(bir, effects, chains, body);
    let stopped = chain.last().filter(|s| s.kind == StepKind::Ambiguous);
    let path: Vec<&str> = chain
        .iter()
        .filter(|s| s.kind != StepKind::Ambiguous)
        .map(|s| s.name.as_str())
        .collect();
    if path.len() > 1 || stopped.is_some() {
        message = format!("{} via `{}`", message, render_path(&path));
    }
    if let Some(step) = stopped {
        message = format!(
            "{} (further steps are ambiguous: several functions are named `{}`)",
            message, step.name
        );
    }

    let anchor = chain
        .iter()
        .rposition(|s| s.span.is_some())
        .filter(|i| *i > 0);
    let primary = anchor.map_or(body.span, |i| {
        chain[i].span.expect("rposition found a span")
    });
    let indirect = chain.iter().any(|s| s.kind == StepKind::Indirect);

    let mut diag = BirDiagnostic::new("E0727", "[nogc]", primary, message);
    if indirect {
        diag = diag.with_hint("effects assumed to reach managed memory here".to_string());
    }
    for step in capped_labels(&chain[..anchor.unwrap_or(0)]) {
        if let Some(span) = step.span {
            diag = diag.with_secondary(span, step_label(step));
        }
    }
    if indirect {
        diag = diag.with_note(
            "the call target here is not statically known, so its effects are conservatively \
             assumed to reach managed memory"
                .to_string(),
        );
    }
    if chain.len() == 1 && bir.bodies.iter().filter(|b| b.name == body.name).count() > 1 {
        diag = diag.with_note(format!(
            "several functions in this program are named `{}`, and the nogc check merges their \
             effects",
            body.name
        ));
    }
    if indirect {
        return diag.with_help(format!(
            "call a function by name so its effects can be checked, or drop `nogc` from `{}`",
            body.name
        ));
    }
    diag.with_help(format!(
        "keep the value on the stack (an array or a `&[T]` slice), make every function on this \
         path nogc, or drop `nogc` from `{}`",
        body.name
    ))
}

fn render_path(path: &[&str]) -> String {
    if path.len() <= CHAIN_HEAD + CHAIN_TAIL + 1 {
        return path.join(" -> ");
    }
    format!(
        "{} -> ... ({} more) ... -> {}",
        path[..CHAIN_HEAD].join(" -> "),
        path.len() - CHAIN_HEAD - CHAIN_TAIL,
        path[path.len() - CHAIN_TAIL..].join(" -> ")
    )
}

fn capped_labels(steps: &[super::effects::Step]) -> Vec<&super::effects::Step> {
    if steps.len() <= MAX_SECONDARY {
        return steps.iter().collect();
    }
    std::iter::once(&steps[0])
        .chain(steps[steps.len() - (MAX_SECONDARY - 1)..].iter())
        .collect()
}

fn step_label(step: &super::effects::Step) -> String {
    use super::effects::StepKind;
    match step.kind {
        StepKind::Root => format!("`{}` is declared nogc here", step.name),
        StepKind::Callee => format!("calls `{}` here", step.name),
        StepKind::Operation => format!("`{}` reaches managed memory here", step.name),
        StepKind::Ambiguous => format!("`{}` is not a single function", step.name),
        StepKind::Indirect => "the callee here is unknown".to_string(),
    }
}

struct UseSite {
    local: BirLocalId,
    is_move: bool,
}

fn is_affine(body: &BirBody, l: BirLocalId) -> bool {
    body.locals[l.0 as usize].category.is_affine()
}

fn rvalue_uses(rv: &BirRvalue, out: &mut Vec<UseSite>) {
    let op = |o: &BirOperand, out: &mut Vec<UseSite>| match o {
        BirOperand::Move(p) => out.push(UseSite {
            local: p.local,
            is_move: p.proj.is_empty(),
        }),
        BirOperand::Copy(p) => out.push(UseSite {
            local: p.local,
            is_move: false,
        }),
        BirOperand::Const => {}
    };
    match rv {
        BirRvalue::Use(o) | BirRvalue::UnOp(o) => op(o, out),
        BirRvalue::BinOp(a, b) => {
            op(a, out);
            op(b, out);
        }
        BirRvalue::Aggregate(v) | BirRvalue::Call { args: v, .. } => {
            for o in v {
                op(o, out);
            }
        }
        // a borrow is a non-consuming read of its place
        BirRvalue::Ref { place, .. } | BirRvalue::Reborrow { place, .. } => out.push(UseSite {
            local: place.local,
            is_move: false,
        }),
    }
}

fn term_uses(term: &BirTerminator, out: &mut Vec<UseSite>) {
    let op = |o: &BirOperand, out: &mut Vec<UseSite>| match o {
        BirOperand::Move(p) => out.push(UseSite {
            local: p.local,
            is_move: p.proj.is_empty(),
        }),
        BirOperand::Copy(p) => out.push(UseSite {
            local: p.local,
            is_move: false,
        }),
        BirOperand::Const => {}
    };
    match term {
        BirTerminator::Branch { discr, .. } => op(discr, out),
        BirTerminator::Return(Some(o)) => op(o, out),
        _ => {}
    }
}

fn apply_stmt(body: &BirBody, st: &mut [Lat], stmt: &BirStmt) {
    match &stmt.kind {
        BirStmtKind::Assign { dest, rvalue } => {
            let mut uses = Vec::new();
            rvalue_uses(rvalue, &mut uses);
            for u in &uses {
                if u.is_move && is_affine(body, u.local) {
                    st[u.local.0 as usize] = Lat::Moved;
                }
            }
            if dest.proj.is_empty() && is_affine(body, dest.local) {
                st[dest.local.0 as usize] = Lat::Live;
            }
        }
        BirStmtKind::StorageLive(l) => st[l.0 as usize] = Lat::Uninit,
        BirStmtKind::StorageDead(l) => st[l.0 as usize] = Lat::Uninit,
        BirStmtKind::Drop(_) => {}
    }
}

fn apply_term(body: &BirBody, st: &mut [Lat], term: &BirTerminator) {
    let mut uses = Vec::new();
    term_uses(term, &mut uses);
    for u in &uses {
        if u.is_move && is_affine(body, u.local) {
            st[u.local.0 as usize] = Lat::Moved;
        }
    }
}

fn successors(term: &BirTerminator) -> Vec<BirBlockId> {
    match term {
        BirTerminator::Goto(t) => vec![*t],
        BirTerminator::Branch { targets, .. } => targets.clone(),
        BirTerminator::Return(_) | BirTerminator::Unreachable => Vec::new(),
    }
}

fn check_body(
    body: &BirBody,
    errors: &mut Vec<BirDiagnostic>,
    drops: &mut HashMap<DropKey, Vec<DropKey>>,
) {
    let n_locals = body.locals.len();
    let mut index: HashMap<u32, usize> = HashMap::new();
    for (i, b) in body.blocks.iter().enumerate() {
        index.insert(b.id.0, i);
    }
    let nblocks = body.blocks.len();

    let mut entry = vec![Lat::Uninit; n_locals];
    for i in 0..body.arg_count.min(n_locals) {
        if body.locals[i].category.is_affine() {
            entry[i] = Lat::Live;
        }
    }

    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nblocks];
    for (i, b) in body.blocks.iter().enumerate() {
        for s in successors(&b.term) {
            if let Some(&j) = index.get(&s.0) {
                preds[j].push(i);
            }
        }
    }
    let entry_idx = *index.get(&body.entry.0).unwrap_or(&0);

    let mut in_state: Vec<Option<Vec<Lat>>> = vec![None; nblocks];
    let mut out_state: Vec<Option<Vec<Lat>>> = vec![None; nblocks];

    // round-robin to a fixpoint; small cfgs, monotone raise to maybe, so it terminates
    let mut changed = true;
    while changed {
        changed = false;
        for i in 0..nblocks {
            let new_in = if i == entry_idx {
                Some(entry.clone())
            } else {
                let mut acc: Option<Vec<Lat>> = None;
                for &p in &preds[i] {
                    if let Some(po) = &out_state[p] {
                        acc = Some(match acc {
                            None => po.clone(),
                            Some(a) => join_state(&a, po),
                        });
                    }
                }
                acc
            };
            let Some(new_in) = new_in else {
                continue;
            };
            if in_state[i].as_ref() != Some(&new_in) {
                in_state[i] = Some(new_in.clone());
                changed = true;
            }
            let mut st = new_in;
            for stmt in &body.blocks[i].stmts {
                apply_stmt(body, &mut st, stmt);
            }
            apply_term(body, &mut st, &body.blocks[i].term);
            if out_state[i].as_ref() != Some(&st) {
                out_state[i] = Some(st);
                changed = true;
            }
        }
    }

    for i in 0..nblocks {
        let Some(in_i) = &in_state[i] else {
            continue;
        };
        let mut st = in_i.clone();
        let mut moved_at: Vec<Option<Span>> = vec![None; n_locals];
        for l in 0..n_locals {
            if matches!(st[l], Lat::Moved | Lat::Maybe) {
                moved_at[l] = first_move_span(body, BirLocalId(l as u32));
            }
        }
        for stmt in &body.blocks[i].stmts {
            if let BirStmtKind::Assign { dest, rvalue } = &stmt.kind {
                let mut uses = Vec::new();
                rvalue_uses(rvalue, &mut uses);
                for u in &uses {
                    if is_affine(body, u.local) {
                        check_use(body, &st, u, stmt.span, &moved_at, errors);
                        if u.is_move {
                            st[u.local.0 as usize] = Lat::Moved;
                            moved_at[u.local.0 as usize] = Some(stmt.span);
                        }
                    }
                }
                if dest.proj.is_empty() && is_affine(body, dest.local) {
                    st[dest.local.0 as usize] = Lat::Live;
                    moved_at[dest.local.0 as usize] = None;
                }
            } else {
                apply_stmt(body, &mut st, stmt);
            }
        }
        let mut tuses = Vec::new();
        term_uses(&body.blocks[i].term, &mut tuses);
        for u in &tuses {
            if is_affine(body, u.local) {
                check_use(body, &st, u, body.blocks[i].term_span, &moved_at, errors);
            }
        }
    }

    if body.is_toplevel {
        return;
    }

    for se in &body.scope_exits {
        let Some(idx) = index.get(&se.exit_block.0) else {
            continue;
        };
        let Some(in_b) = &in_state[*idx] else {
            continue;
        };
        let st = state_before(body, in_b, *idx, se.exit_index);
        for l in &se.locals {
            match st[l.0 as usize] {
                Lat::Live => record_drop(drops, se.scope_span, decl_span(body, *l)),
                Lat::Maybe => errors.push(maybe_scope_diag(body, *l, se.scope_span)),
                _ => {}
            }
        }
    }

    for rp in &body.returns {
        let Some(idx) = index.get(&rp.block.0) else {
            continue;
        };
        let Some(out_b) = &out_state[*idx] else {
            continue;
        };
        for l in &rp.in_scope {
            match out_b[l.0 as usize] {
                Lat::Live => record_drop(drops, rp.span, decl_span(body, *l)),
                Lat::Maybe => errors.push(maybe_scope_diag(body, *l, rp.span)),
                _ => {}
            }
        }
    }

    for ra in &body.reassigns {
        let Some(idx) = index.get(&ra.block.0) else {
            continue;
        };
        let Some(in_b) = &in_state[*idx] else {
            continue;
        };
        let pre = state_before(body, in_b, *idx, ra.stmt_index);
        match pre[ra.local.0 as usize] {
            Lat::Live => record_drop(drops, ra.span, decl_span(body, ra.local)),
            Lat::Maybe => errors.push(
                BirDiagnostic::new(
                    "E0704",
                    "[move]",
                    ra.span,
                    format!(
                        "[move] the old value of `{}` may have been moved on an earlier branch; \
                         it cannot be deterministically destroyed on reassignment (conditional moves \
                         are not supported yet)",
                        local_name(body, ra.local)
                    ),
                )
                .with_secondary(decl_span(body, ra.local), declared_here(body, ra.local)),
            ),
            _ => {}
        }
    }
}

fn state_before(body: &BirBody, in_b: &[Lat], block_idx: usize, upto: usize) -> Vec<Lat> {
    let mut st = in_b.to_vec();
    for stmt in body.blocks[block_idx].stmts.iter().take(upto) {
        apply_stmt(body, &mut st, stmt);
    }
    st
}

fn record_drop(drops: &mut HashMap<DropKey, Vec<DropKey>>, point: Span, decl: Span) {
    drops
        .entry(drop_key(&point))
        .or_default()
        .push(drop_key(&decl));
}

fn decl_span(body: &BirBody, l: BirLocalId) -> Span {
    body.locals[l.0 as usize].decl_span
}

fn local_name(body: &BirBody, l: BirLocalId) -> String {
    body.locals[l.0 as usize]
        .name
        .clone()
        .unwrap_or_else(|| "value".to_string())
}

fn first_move_span(body: &BirBody, target: BirLocalId) -> Option<Span> {
    for b in &body.blocks {
        for stmt in &b.stmts {
            if let BirStmtKind::Assign { rvalue, .. } = &stmt.kind {
                let mut uses = Vec::new();
                rvalue_uses(rvalue, &mut uses);
                if uses.iter().any(|u| u.is_move && u.local == target) {
                    return Some(stmt.span);
                }
            }
        }
    }
    None
}

fn check_use(
    body: &BirBody,
    st: &[Lat],
    u: &UseSite,
    use_span: Span,
    moved_at: &[Option<Span>],
    errors: &mut Vec<BirDiagnostic>,
) {
    let name = local_name(body, u.local);
    let moved_span = moved_at[u.local.0 as usize];
    match st[u.local.0 as usize] {
        Lat::Moved => {
            if u.is_move {
                let mut d = BirDiagnostic::new(
                    "E0702",
                    "[move]",
                    use_span,
                    format!("[move] `{name}` is moved here, but it was already moved"),
                );
                if let Some(s) = moved_span {
                    d = d.with_secondary(s, "first moved here".to_string());
                }
                errors.push(d);
            } else {
                let mut d = BirDiagnostic::new(
                    "E0701",
                    "[move]",
                    use_span,
                    format!("[move] `{name}` is used here after it was moved"),
                );
                if let Some(s) = moved_span {
                    d = d.with_secondary(s, "moved here".to_string());
                }
                errors.push(d);
            }
        }
        Lat::Maybe => {
            let mut d = BirDiagnostic::new(
                "E0703",
                "[move]",
                use_span,
                format!(
                    "[move] `{name}` is used here after it may have been moved on an earlier branch"
                ),
            );
            if let Some(s) = moved_span {
                d = d.with_secondary(s, "moved here on one branch".to_string());
            }
            errors.push(d);
        }
        Lat::Uninit | Lat::Live => {}
    }
}

fn maybe_scope_diag(body: &BirBody, l: BirLocalId, primary: Span) -> BirDiagnostic {
    BirDiagnostic::new("E0704", "[move]", primary, maybe_scope_msg(body, l))
        .with_secondary(decl_span(body, l), declared_here(body, l))
}

fn declared_here(body: &BirBody, l: BirLocalId) -> String {
    format!("`{}` declared here", local_name(body, l))
}

fn maybe_scope_msg(body: &BirBody, l: BirLocalId) -> String {
    format!(
        "[move] `{}` may have been moved on an earlier branch; it cannot be deterministically \
         destroyed at scope end (conditional moves are not supported yet)",
        local_name(body, l)
    )
}

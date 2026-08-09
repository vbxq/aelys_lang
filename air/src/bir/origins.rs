// per-body return-origin summaries + the d1 return-ref-to-local escape.
// a seeded holds fixpoint over origin = param(i) | loan(id): the param seed is what lets a

use std::collections::{HashMap, HashSet};

use super::*;

// per parameter index (< arg_count): true iff the returned reference may borrow that parameter,
pub struct ReturnOrigins {
    pub params: Vec<bool>,
    // (floor): the body is rejected, so its call sites over-approximate rather than trust params
    pub escapes_local: bool,
}

pub enum SummaryEntry {
    Unique(ReturnOrigins),
    Ambiguous,
}

pub type Summaries = HashMap<String, SummaryEntry>;

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Origin {
    Param(usize),
    Loan(u32),
}

pub fn summaries(bir: &BirProgram, errors: &mut Vec<BirDiagnostic>) -> Summaries {
    let mut map: Summaries = HashMap::new();
    for body in &bir.bodies {
        let ro = summarize_body(body, errors);
        map.entry(body.name.clone())
            .and_modify(|e| *e = SummaryEntry::Ambiguous)
            .or_insert(SummaryEntry::Unique(ro));
    }
    map
}

pub enum ParamWrites {
    Unique(Vec<bool>),
    Ambiguous,
}

pub type SliceWrites = HashMap<String, ParamWrites>;

pub fn slice_param_writes(bir: &BirProgram) -> SliceWrites {
    let mut map: SliceWrites = HashMap::new();
    for body in &bir.bodies {
        let w = body_slice_param_writes(body);
        map.entry(body.name.clone())
            .and_modify(|e| *e = ParamWrites::Ambiguous)
            .or_insert(ParamWrites::Unique(w));
    }
    map
}

fn body_slice_param_writes(body: &BirBody) -> Vec<bool> {
    let n = body.locals.len();
    let arg_count = body.arg_count.min(n);
    let mut written = vec![false; body.arg_count];
    let slice_params: Vec<usize> = (0..arg_count)
        .filter(|i| matches!(body.locals[*i].ty, InferType::Slice { .. }))
        .collect();
    if slice_params.is_empty() {
        return written;
    }

    let mut sink = Vec::new();
    let loans = loans::gen_loans(body, &mut sink);
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for block in &body.blocks {
        for stmt in &block.stmts {
            if let BirStmtKind::Assign { dest, rvalue } = &stmt.kind {
                if dest.proj.is_empty() {
                    if let BirRvalue::Use(BirOperand::Copy(s) | BirOperand::Move(s)) = rvalue {
                        if s.proj.is_empty() {
                            edges.push((dest.local.0 as usize, s.local.0 as usize));
                        }
                    }
                }
            }
        }
    }
    for l in &loans {
        edges.push((l.holder.0 as usize, l.place.local.0 as usize));
    }

    let mut derived: Vec<HashSet<usize>> = vec![HashSet::new(); n];
    for i in &slice_params {
        derived[*i].insert(*i);
    }
    let mut changed = true;
    while changed {
        changed = false;
        for &(dst, src) in &edges {
            if dst >= n || src >= n || dst == src {
                continue;
            }
            let src_ids: Vec<usize> = derived[src].iter().copied().collect();
            for id in src_ids {
                if derived[dst].insert(id) {
                    changed = true;
                }
            }
        }
    }

    for block in &body.blocks {
        for stmt in &block.stmts {
            let BirStmtKind::Assign { dest, rvalue } = &stmt.kind else {
                continue;
            };
            if !dest.proj.is_empty() {
                for p in &derived[dest.local.0 as usize] {
                    written[*p] = true;
                }
            }
// a one-level closure, not a call-graph fixpoint: any callee handed the view writes
            if let BirRvalue::Call { args, .. } = rvalue {
                for arg in args {
                    if let BirOperand::Copy(p) | BirOperand::Move(p) = arg {
                        if p.proj.is_empty() {
                            for q in &derived[p.local.0 as usize] {
                                written[*q] = true;
                            }
                        }
                    }
                }
            }
        }
    }
    written
}

fn summarize_body(body: &BirBody, errors: &mut Vec<BirDiagnostic>) -> ReturnOrigins {
    let arg_count = body.arg_count;
    let mut params = vec![false; arg_count];
    let mut escapes_local = false;

    // gate on the declared return type: a non-reference return borrows nothing, so even a
    if !is_ref_ty(&body.return_type) {
        return ReturnOrigins {
            params,
            escapes_local,
        };
    }

    let mut sink = Vec::new();
    let loans = loans::gen_loans(body, &mut sink);
    let holds = compute_origin_holds(body, &loans);

    for block in &body.blocks {
        let BirTerminator::Return(Some(op)) = &block.term else {
            continue;
        };
        let ret_span = block.term_span;
        let origin_set = match operand_base(op) {
            Some(base) => holds.get(base.0 as usize).cloned().unwrap_or_default(),
            None => HashSet::new(),
        };
        if origin_set.is_empty() {
            // has no call edges) cannot be proven loan-free, so reject conservatively (the floor)
            escapes_local = true;
            errors.push(BirDiagnostic::new(
                "E0723",
                "[escape]",
                ret_span,
                floor_message(),
            ));
            continue;
        }
        for origin in &origin_set {
            match origin {
                Origin::Param(i) => params[*i] = true,
                Origin::Loan(id) => {
                    let root = loans[*id as usize].place.local;
                    let root_is_ref = is_ref_ty(&body.locals[root.0 as usize].ty);
                    // the guard mirrors its twin below: a by-value parameter is storage the
                    // callee owns and destroys on return, so a reference into it dangles just
                    if (root.0 as usize) < arg_count && root_is_ref {
                        params[root.0 as usize] = true;
                    } else if root_is_ref && !holds[root.0 as usize].is_empty() {
                    } else {
                        // the returned reference borrows a local that dies on return: d1
                        escapes_local = true;
                        let decl = body.locals[root.0 as usize].decl_span;
                        errors.push(
                            BirDiagnostic::new(
                                "E0721",
                                "[escape]",
                                ret_span,
                                d1_message(body, root),
                            )
                            .with_secondary(
                                decl,
                                format!(
                                    "`{}` declared here; destroyed on return",
                                    local_name(body, root)
                                ),
                            ),
                        );
                    }
                }
            }
        }
    }

    ReturnOrigins {
        params,
        escapes_local,
    }
}

// holds[local] = the origins the local may carry. flow-insensitive union to fixpoint, over the
// exact inference edge set (whole-local copy/move + reborrow inheritance) and no call edges.
fn compute_origin_holds(body: &BirBody, loans: &[loans::Loan]) -> Vec<HashSet<Origin>> {
    let n = body.locals.len();
    let mut holds: Vec<HashSet<Origin>> = vec![HashSet::new(); n];

    for i in 0..body.arg_count.min(n) {
        if is_ref_ty(&body.locals[i].ty) {
            holds[i].insert(Origin::Param(i));
        }
    }
    for l in loans {
        if (l.holder.0 as usize) < n {
            holds[l.holder.0 as usize].insert(Origin::Loan(l.id.0));
        }
    }

    let mut edges: Vec<(usize, usize)> = Vec::new();
    for block in &body.blocks {
        for stmt in &block.stmts {
            if let BirStmtKind::Assign { dest, rvalue } = &stmt.kind {
                if dest.proj.is_empty() {
                    if let BirRvalue::Use(BirOperand::Copy(s) | BirOperand::Move(s)) = rvalue {
                        if s.proj.is_empty() {
                            edges.push((dest.local.0 as usize, s.local.0 as usize));
                        }
                    }
                }
            }
        }
    }
    for l in loans {
        if let Some(r) = l.reborrow_base {
            edges.push((l.holder.0 as usize, r.0 as usize));
        }
        if is_ref_ty(&body.locals[l.place.local.0 as usize].ty) {
            edges.push((l.holder.0 as usize, l.place.local.0 as usize));
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for &(dst, src) in &edges {
            if dst >= n || src >= n || dst == src {
                continue;
            }
            let src_ids: Vec<Origin> = holds[src].iter().copied().collect();
            for id in src_ids {
                if holds[dst].insert(id) {
                    changed = true;
                }
            }
        }
    }
    holds
}

fn operand_base(op: &BirOperand) -> Option<BirLocalId> {
    match op {
        BirOperand::Copy(p) | BirOperand::Move(p) => Some(p.local),
        BirOperand::Const => None,
    }
}

fn local_name(body: &BirBody, local: BirLocalId) -> String {
    body.locals
        .get(local.0 as usize)
        .and_then(|l| l.name.clone())
        .unwrap_or_else(|| "value".to_string())
}

fn d1_message(body: &BirBody, local: BirLocalId) -> String {
    let name = body
        .locals
        .get(local.0 as usize)
        .and_then(|l| l.name.clone())
        .unwrap_or_else(|| "value".to_string());
    format!(
        "[escape] function returns a reference to local `{name}`, which is destroyed when the \
         function returns"
    )
}

fn floor_message() -> String {
    "[escape] cannot infer the origin of this returned reference".to_string()
}


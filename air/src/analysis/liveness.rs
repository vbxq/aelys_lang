use crate::analysis::cfg;
use crate::{AirFunction, AirStmtKind, BlockId, Callee, LocalId, Operand};
use std::collections::{HashMap, HashSet};

pub struct Liveness {
    pub live_in: HashMap<BlockId, HashSet<LocalId>>,
    pub live_out: HashMap<BlockId, HashSet<LocalId>>,
}

impl Liveness {
    pub fn compute(function: &AirFunction) -> Liveness {
        compute_with_mask(function, &|_, _, _| false)
    }
}

// a local's own releases are exactly what elision deletes, so they must not keep it live
pub fn is_release_of(kind: &AirStmtKind, local: LocalId) -> bool {
    if let AirStmtKind::CallVoid {
        func: Callee::Named(name),
        args,
    } = kind
    {
        if name == "__aelys_rc_release" && args.len() == 1 {
            if let Operand::Copy(l) | Operand::Move(l) = &args[0] {
                return *l == local;
            }
        }
    }
    false
}

pub fn dead_after(function: &AirFunction, bid: BlockId, idx: usize, local: LocalId) -> bool {
    let mask = |_b: BlockId, k: &AirStmtKind, l: LocalId| -> bool { is_release_of(k, l) };
    let live = compute_one_local_with_mask(function, local, &mask);

    let idx_in_map = cfg::block_index_map(function);
    let bpos = match idx_in_map.get(&bid) {
        Some(p) => *p,
        None => return false, // unknown block, stay conservative and treat it as live
    };
    let block = &function.blocks[bpos];

    for stmt in block.stmts.iter().skip(idx + 1) {
        if mask(bid, &stmt.kind, local) {
            continue;
        }
        let mut used = false;
        cfg::for_each_use_stmt(&stmt.kind, |u| {
            if u == local {
                used = true;
            }
        });
        if used {
            return false;
        }
    }
    {
        let mut used = false;
        cfg::for_each_use_term(&block.terminator, |u| {
            if u == local {
                used = true;
            }
        });
        if used {
            return false;
        }
    }
    for succ in cfg::successors(&block.terminator) {
        if live.get(&succ).map(|s| s.contains(&local)).unwrap_or(false) {
            return false;
        }
    }
    true
}

fn compute_with_mask(
    function: &AirFunction,
    mask: &dyn Fn(BlockId, &AirStmtKind, LocalId) -> bool,
) -> Liveness {
    let preds = cfg::predecessors(function);
    let mut live_in: HashMap<BlockId, HashSet<LocalId>> = function
        .blocks
        .iter()
        .map(|b| (b.id, HashSet::new()))
        .collect();
    let mut live_out: HashMap<BlockId, HashSet<LocalId>> = function
        .blocks
        .iter()
        .map(|b| (b.id, HashSet::new()))
        .collect();

    let mut worklist: Vec<BlockId> = function.blocks.iter().map(|b| b.id).collect();
    while let Some(bid) = worklist.pop() {
        let bpos = function.blocks.iter().position(|b| b.id == bid).unwrap();
        let block = &function.blocks[bpos];

        let mut out: HashSet<LocalId> = HashSet::new();
        for succ in cfg::successors(&block.terminator) {
            if let Some(s) = live_in.get(&succ) {
                out.extend(s.iter().copied());
            }
        }

        let new_in = transfer_block(block, &out, mask);

        let changed = live_in.get(&bid).map(|cur| cur != &new_in).unwrap_or(true);
        live_out.insert(bid, out);
        if changed {
            live_in.insert(bid, new_in);
            if let Some(ps) = preds.get(&bid) {
                for p in ps {
                    worklist.push(*p);
                }
            }
        }
    }

    Liveness { live_in, live_out }
}

fn compute_one_local_with_mask(
    function: &AirFunction,
    local: LocalId,
    mask: &dyn Fn(BlockId, &AirStmtKind, LocalId) -> bool,
) -> HashMap<BlockId, HashSet<LocalId>> {
    // the mask already only ever suppresses `local`, so the whole-function fixpoint suffices
    let _ = local;
    compute_with_mask(function, mask).live_in
}

fn transfer_block(
    block: &crate::AirBlock,
    live_out: &HashSet<LocalId>,
    mask: &dyn Fn(BlockId, &AirStmtKind, LocalId) -> bool,
) -> HashSet<LocalId> {
    let mut live = live_out.clone();

    cfg::for_each_use_term(&block.terminator, |u| {
        live.insert(u);
    });

    for stmt in block.stmts.iter().rev() {
        if let Some(def) = stmt_def(&stmt.kind) {
            live.remove(&def);
        }
        // uses regenerate after the kill, so a self-referential `%a = f(%a)` keeps %a live
        cfg::for_each_use_stmt(&stmt.kind, |u| {
            if !mask(block.id, &stmt.kind, u) {
                live.insert(u);
            }
        });
    }

    live
}

// field/deref/index destinations define no fresh local, they mutate through an existing base
fn stmt_def(kind: &AirStmtKind) -> Option<LocalId> {
    match kind {
        AirStmtKind::Assign {
            place: crate::Place::Local(l),
            ..
        } => Some(*l),
        AirStmtKind::GcAlloc { local, .. }
        | AirStmtKind::Alloc { local, .. }
        | AirStmtKind::RcAlloc { local, .. } => Some(*local),
        _ => None,
    }
}


use crate::analysis::{escape, liveness};
use crate::{
    AirFunction, AirProgram, AirStmtKind, BlockId, Callee, LocalId, Operand, Place, Rvalue,
};
use std::collections::HashMap;

const RETAIN: &str = "__aelys_rc_retain";
const RELEASE: &str = "__aelys_rc_release";

pub fn eliminate_redundant_rc(program: &mut AirProgram) {
    if std::env::var("AELYS_RC_ELISION").as_deref() == Ok("0") {
        return;
    }
    for function in &mut program.functions {
        eliminate_function_redundant_rc(function);
    }
}

struct Elision {
    retain_pos: (BlockId, usize),
    release_positions: Vec<(BlockId, usize)>,
}

fn eliminate_function_redundant_rc(function: &mut AirFunction) {
    let aggregate_set = escape::stored_into_aggregate_set(function);
    let writes = collect_write_counts(function);
    let releases = collect_release_positions(function);

    let mut elisions: Vec<Elision> = Vec::new();

    for block in &function.blocks {
        for (idx, stmt) in block.stmts.iter().enumerate() {
            let Some((b_local, a_local)) = clone_assign_locals(&stmt.kind) else {
                continue;
            };
            if idx == 0 {
                continue; // no preceding statement to be the clone retain
            }
            if a_local == b_local {
                continue;
            }

            let retain_stmt = &block.stmts[idx - 1];
            if !is_retain_of(&retain_stmt.kind, a_local) {
                continue;
            }

            let rel_a = releases.get(&a_local).cloned().unwrap_or_default();
            let rel_b = releases.get(&b_local).cloned().unwrap_or_default();
            if rel_a.is_empty() || rel_b.is_empty() {
                continue;
            }

            if aggregate_set.contains(&a_local) {
                continue;
            }
            if writes.get(&a_local).copied().unwrap_or(0) != 1
                || writes.get(&b_local).copied().unwrap_or(0) != 1
            {
                continue;
            }
            // a reassignment could clobber the inherited +1 and break single-owner-ness
            if local_is_mut(function, a_local) || local_is_mut(function, b_local) {
                continue;
            }

            if escape::escapes(function, a_local).is_some() {
                continue;
            }

            if !liveness::dead_after(function, block.id, idx, a_local) {
                continue;
            }

            // if a release(%a) is ever alone on a path, eliding would leak or double-free
            let rel_b_blocks: std::collections::HashSet<BlockId> =
                rel_b.iter().map(|(bid, _)| *bid).collect();
            let co_located = rel_a.iter().all(|(bid, _)| rel_b_blocks.contains(bid));
            if !co_located {
                continue;
            }

            elisions.push(Elision {
                retain_pos: (block.id, idx - 1),
                release_positions: rel_a,
            });
        }
    }

    if elisions.is_empty() {
        return;
    }

    let mut to_delete: HashMap<BlockId, std::collections::BTreeSet<usize>> = HashMap::new();
    for e in &elisions {
        let (bid, ridx) = e.retain_pos;
        to_delete.entry(bid).or_default().insert(ridx);
        for (rbid, ridx) in &e.release_positions {
            to_delete.entry(*rbid).or_default().insert(*ridx);
        }
    }

    for block in &mut function.blocks {
        if let Some(indices) = to_delete.get(&block.id) {
            let mut keep = Vec::with_capacity(block.stmts.len());
            for (i, stmt) in block.stmts.drain(..).enumerate() {
                if !indices.contains(&i) {
                    keep.push(stmt);
                }
            }
            block.stmts = keep;
        }
    }
}

fn clone_assign_locals(kind: &AirStmtKind) -> Option<(LocalId, LocalId)> {
    match kind {
        AirStmtKind::Assign {
            place: Place::Local(b),
            rvalue: Rvalue::Use(Operand::Copy(a) | Operand::Move(a)),
        } => Some((*b, *a)),
        _ => None,
    }
}

fn is_retain_of(kind: &AirStmtKind, local: LocalId) -> bool {
    matches_rc_call(kind, RETAIN, local)
}

fn matches_rc_call(kind: &AirStmtKind, name: &str, local: LocalId) -> bool {
    if let AirStmtKind::CallVoid {
        func: Callee::Named(n),
        args,
    } = kind
    {
        if n == name && args.len() == 1 {
            if let Operand::Copy(l) | Operand::Move(l) = &args[0] {
                return *l == local;
            }
        }
    }
    false
}

fn collect_release_positions(function: &AirFunction) -> HashMap<LocalId, Vec<(BlockId, usize)>> {
    let mut map: HashMap<LocalId, Vec<(BlockId, usize)>> = HashMap::new();
    for block in &function.blocks {
        for (idx, stmt) in block.stmts.iter().enumerate() {
            if let AirStmtKind::CallVoid {
                func: Callee::Named(n),
                args,
            } = &stmt.kind
            {
                if n == RELEASE && args.len() == 1 {
                    if let Operand::Copy(l) | Operand::Move(l) = &args[0] {
                        map.entry(*l).or_default().push((block.id, idx));
                    }
                }
            }
        }
    }
    map
}

// a field/index store counts as a write of the base local, which is conservative here
fn collect_write_counts(function: &AirFunction) -> HashMap<LocalId, u32> {
    let mut counts: HashMap<LocalId, u32> = HashMap::new();
    let bump = |l: LocalId, m: &mut HashMap<LocalId, u32>| {
        *m.entry(l).or_insert(0) += 1;
    };
    for block in &function.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                AirStmtKind::Assign { place, .. } => match place {
                    Place::Local(l) | Place::Field(l, _) | Place::Index(l, _) => bump(*l, &mut counts),
                    Place::Deref(_) => {}
                },
                AirStmtKind::GcAlloc { local, .. }
                | AirStmtKind::Alloc { local, .. }
                | AirStmtKind::RcAlloc { local, .. } => bump(*local, &mut counts),
                _ => {}
            }
        }
        if let crate::AirTerminator::Invoke { ret, .. } = &block.terminator {
            match ret {
                Place::Local(l) | Place::Field(l, _) | Place::Index(l, _) => bump(*l, &mut counts),
                Place::Deref(_) => {}
            }
        }
    }
    counts
}

fn local_is_mut(function: &AirFunction, local: LocalId) -> bool {
    function
        .locals
        .iter()
        .find(|l| l.id == local)
        .map(|l| l.is_mut)
        .unwrap_or(false)
}

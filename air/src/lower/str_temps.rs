use std::collections::{HashMap, HashSet};

use crate::{
    AirBlock, AirFunction, AirLocal, AirStmt, AirStmtKind, AirTerminator, AirType, BinOp, Callee,
    LocalId, Operand, Place, Rvalue,
};

const STR_RETAIN: &str = "__aelys_str_retain";
const STR_RELEASE: &str = "__aelys_str_release";

fn operand_local(op: &Operand) -> Option<LocalId> {
    match op {
        Operand::Copy(id) | Operand::Move(id) => Some(*id),
        Operand::Const(_) => None,
    }
}

fn push_operand(out: &mut Vec<LocalId>, op: &Operand) {
    if let Some(id) = operand_local(op) {
        out.push(id);
    }
}

fn push_place(out: &mut Vec<LocalId>, place: &Place) {
    match place {
        Place::Local(_) | Place::Global(_) => {}
        Place::Field(base, _) | Place::Deref(base) => out.push(*base),
        Place::Index(base, index) => {
            out.push(*base);
            push_operand(out, index);
        }
    }
}

fn push_callee(out: &mut Vec<LocalId>, func: &Callee, args: &[Operand]) {
    if let Callee::FnPtr(id) = func {
        out.push(*id);
    }
    for arg in args {
        push_operand(out, arg);
    }
}

fn stmt_reads(kind: &AirStmtKind) -> Vec<LocalId> {
    let mut out = Vec::new();
    match kind {
        AirStmtKind::Assign { place, rvalue } => {
            push_place(&mut out, place);
            match rvalue {
                Rvalue::Use(op) | Rvalue::UnaryOp(_, op) | Rvalue::Deref(op) | Rvalue::Len(op) => {
                    push_operand(&mut out, op)
                }
                Rvalue::Cast { operand, .. }
                | Rvalue::EnumTag { operand, .. }
                | Rvalue::EnumPayload { operand, .. } => push_operand(&mut out, operand),
                Rvalue::FieldAccess { base, .. } => push_operand(&mut out, base),
                Rvalue::BinaryOp(_, left, right) => {
                    push_operand(&mut out, left);
                    push_operand(&mut out, right);
                }
                Rvalue::Index { base, index } => {
                    push_operand(&mut out, base);
                    push_operand(&mut out, index);
                }
                Rvalue::AddressOf(place) => match place {
                    Place::Local(base) => out.push(*base),
                    other => push_place(&mut out, other),
                },
                Rvalue::Call { func, args } => push_callee(&mut out, func, args),
                Rvalue::StructInit { fields, .. } => {
                    for (_, op) in fields {
                        push_operand(&mut out, op);
                    }
                }
                Rvalue::EnumInit { payload, .. } => {
                    for op in payload {
                        push_operand(&mut out, op);
                    }
                }
                Rvalue::ClosureCreate { env, .. } => push_operand(&mut out, env),
                Rvalue::SliceFromParts { ptr, len } => {
                    push_operand(&mut out, ptr);
                    push_operand(&mut out, len);
                }
            }
        }
        AirStmtKind::CallVoid { func, args } => push_callee(&mut out, func, args),
        AirStmtKind::GcAlloc { local, .. }
        | AirStmtKind::Alloc { local, .. }
        | AirStmtKind::RcAlloc { local, .. }
        | AirStmtKind::GcDrop(local)
        | AirStmtKind::Free(local) => out.push(*local),
        AirStmtKind::ArenaCreate(_)
        | AirStmtKind::ArenaDestroy(_)
        | AirStmtKind::MemoryFence(_) => {}
    }
    out
}

fn terminator_reads(terminator: &AirTerminator) -> Vec<LocalId> {
    let mut out = Vec::new();
    match terminator {
        AirTerminator::Return(Some(op)) => push_operand(&mut out, op),
        AirTerminator::Branch { cond, .. } => push_operand(&mut out, cond),
        AirTerminator::Switch { discr, .. } => push_operand(&mut out, discr),
        AirTerminator::Invoke {
            func, args, ret, ..
        } => {
            push_callee(&mut out, func, args);
            push_place(&mut out, ret);
        }
        AirTerminator::Return(None)
        | AirTerminator::Goto(_)
        | AirTerminator::Unwind
        | AirTerminator::Unreachable
        | AirTerminator::Panic { .. } => {}
    }
    out
}

fn assigned_local(kind: &AirStmtKind) -> Option<LocalId> {
    match kind {
        AirStmtKind::Assign {
            place: Place::Local(dst),
            ..
        }
        | AirStmtKind::GcAlloc { local: dst, .. }
        | AirStmtKind::Alloc { local: dst, .. }
        | AirStmtKind::RcAlloc { local: dst, .. } => Some(*dst),
        _ => None,
    }
}

fn slot_call_arg(kind: &AirStmtKind) -> Option<LocalId> {
    let AirStmtKind::CallVoid {
        func: Callee::Named(name),
        args,
    } = kind
    else {
        return None;
    };
    if name != STR_RETAIN && name != STR_RELEASE {
        return None;
    }
    args.first().and_then(operand_local)
}

fn is_unnamed_str_local(function: &AirFunction, target: LocalId) -> bool {
    function
        .locals
        .iter()
        .any(|local| local.id == target && local.name.is_none() && local.ty == AirType::Str)
}

fn produces_fresh_str(rvalue: &Rvalue, owned_return_fns: &HashSet<String>) -> bool {
    match rvalue {
        Rvalue::Call {
            func: Callee::Named(name),
            ..
        } => {
            crate::symbols::STRING_PRODUCER_SYMBOLS.contains(&name.as_str())
                || owned_return_fns.contains(name)
        }
        Rvalue::BinaryOp(BinOp::Add, _, _) => true,
        _ => false,
    }
}

struct TempUses {
    sites: Vec<(usize, usize)>,
    in_terminator: bool,
}

/// a fresh producer whose value no slot ever owns keeps the +1 it was born with, and nobody frees it
pub(super) fn release_unbound_str_temps(
    function: &mut AirFunction,
    owned_return_fns: &HashSet<String>,
) {
    let mut next_local_id = function
        .locals
        .iter()
        .map(|local| local.id.0)
        .chain(function.params.iter().map(|param| param.id.0))
        .max()
        .map_or(0, |highest| highest + 1);
    let mut slot_args: HashSet<LocalId> = HashSet::new();
    let mut def_count: HashMap<LocalId, usize> = HashMap::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            if let Some(arg) = slot_call_arg(&stmt.kind) {
                slot_args.insert(arg);
            }
            if let Some(dst) = assigned_local(&stmt.kind) {
                *def_count.entry(dst).or_default() += 1;
            }
        }
        if let AirTerminator::Invoke {
            ret: Place::Local(dst),
            ..
        } = &block.terminator
        {
            *def_count.entry(*dst).or_default() += 1;
        }
    }
    let mut accounted: HashSet<LocalId> = HashSet::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            if let AirStmtKind::Assign {
                place: Place::Local(dst),
                rvalue: Rvalue::AddressOf(Place::Local(base)),
            } = &stmt.kind
                && slot_args.contains(dst)
            {
                accounted.insert(*base);
            }
        }
    }

    let mut candidates: Vec<(usize, usize, LocalId)> = Vec::new();
    for (block_index, block) in function.blocks.iter().enumerate() {
        for (stmt_index, stmt) in block.stmts.iter().enumerate() {
            let AirStmtKind::Assign {
                place: Place::Local(target),
                rvalue,
            } = &stmt.kind
            else {
                continue;
            };
            if !produces_fresh_str(rvalue, owned_return_fns)
                || !is_unnamed_str_local(function, *target)
                || accounted.contains(target)
                || def_count.get(target).copied() != Some(1)
            {
                continue;
            }
            candidates.push((block_index, stmt_index, *target));
        }
    }
    if candidates.is_empty() {
        return;
    }

    let wanted: HashSet<LocalId> = candidates.iter().map(|(_, _, t)| *t).collect();
    let mut uses: HashMap<LocalId, TempUses> = HashMap::new();
    for (block_index, block) in function.blocks.iter().enumerate() {
        for (stmt_index, stmt) in block.stmts.iter().enumerate() {
            for id in stmt_reads(&stmt.kind) {
                if !wanted.contains(&id) {
                    continue;
                }
                uses.entry(id)
                    .or_insert(TempUses {
                        sites: Vec::new(),
                        in_terminator: false,
                    })
                    .sites
                    .push((block_index, stmt_index));
            }
        }
        for id in terminator_reads(&block.terminator) {
            if !wanted.contains(&id) {
                continue;
            }
            uses.entry(id)
                .or_insert(TempUses {
                    sites: Vec::new(),
                    in_terminator: false,
                })
                .in_terminator = true;
        }
    }

    let mut inserts: Vec<(usize, usize, LocalId)> = Vec::new();
    for (block_index, def_index, target) in candidates {
        let Some(entry) = uses.get(&target) else {
            continue;
        };
        // a read the release cannot be placed after keeps the temp alive past every candidate point
        if entry.in_terminator || entry.sites.iter().any(|(b, _)| *b != block_index) {
            continue;
        }
        let last = entry
            .sites
            .iter()
            .map(|(_, s)| *s)
            .chain(std::iter::once(def_index))
            .max()
            .unwrap_or(def_index);
        inserts.push((block_index, last + 1, target));
    }
    if inserts.is_empty() {
        return;
    }

    inserts.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
    for (block_index, at, target) in inserts {
        let addr = LocalId(next_local_id);
        next_local_id += 1;
        function.locals.push(AirLocal {
            id: addr,
            ty: AirType::Ptr(Box::new(AirType::Str)),
            name: None,
            is_mut: false,
            span: None,
        });
        let block: &mut AirBlock = &mut function.blocks[block_index];
        let span = block.stmts.get(at.saturating_sub(1)).and_then(|s| s.span);
        block.stmts.insert(
            at,
            AirStmt {
                kind: AirStmtKind::CallVoid {
                    func: Callee::Named(STR_RELEASE.to_string()),
                    args: vec![Operand::Copy(addr)],
                },
                span,
            },
        );
        block.stmts.insert(
            at,
            AirStmt {
                kind: AirStmtKind::Assign {
                    place: Place::Local(addr),
                    rvalue: Rvalue::AddressOf(Place::Local(target)),
                },
                span,
            },
        );
    }
}

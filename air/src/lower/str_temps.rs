use std::collections::{HashMap, HashSet};

use crate::symbols::{STRING_PRODUCER_SYMBOLS, STRING_READER_SYMBOLS};
use crate::{
    AirConst, AirFunction, AirLocal, AirStmt, AirStmtKind, AirTerminator, AirType, BinOp, Callee,
    LocalId, Operand, Place, Rvalue,
};

const TO_STRING: &str = "__aelys_to_string";

#[derive(Clone, Copy, PartialEq, Eq)]
enum Read {
    Consumed,
    Printed,
    Held,
}

fn operand_local(op: &Operand) -> Option<LocalId> {
    match op {
        Operand::Copy(id) | Operand::Move(id) => Some(*id),
        Operand::Const(_) => None,
    }
}

fn note(out: &mut Vec<(LocalId, Read)>, op: &Operand, read: Read) {
    if let Some(id) = operand_local(op) {
        out.push((id, read));
    }
}

fn note_place(out: &mut Vec<(LocalId, Read)>, place: &Place) {
    match place {
        Place::Local(_) | Place::Global(_) => {}
        Place::Field(base, _) | Place::Deref(base) => out.push((*base, Read::Held)),
        Place::Index(base, index) => {
            out.push((*base, Read::Held));
            note(out, index, Read::Held);
        }
    }
}

fn is_print(name: &str) -> bool {
    name == "print" || name == "println"
}

fn note_call(
    out: &mut Vec<(LocalId, Read)>,
    func: &Callee,
    args: &[Operand],
    retaining: &HashSet<String>,
) {
    let read = match func {
        Callee::Named(name) if is_print(name) => Read::Printed,
        Callee::Named(name) if STRING_READER_SYMBOLS.contains(&name.as_str()) => Read::Consumed,
        Callee::Named(name) if retaining.contains(name) => Read::Consumed,
        Callee::FnPtr(_) => Read::Consumed,
        _ => Read::Held,
    };
    if let Callee::FnPtr(id) = func {
        out.push((*id, Read::Held));
    }
    for arg in args {
        note(out, arg, read);
    }
}

fn stmt_reads(kind: &AirStmtKind, retaining: &HashSet<String>) -> Vec<(LocalId, Read)> {
    let mut out = Vec::new();
    match kind {
        AirStmtKind::Assign { place, rvalue } => {
            note_place(&mut out, place);
            match rvalue {
                Rvalue::BinaryOp(_, left, right) => {
                    note(&mut out, left, Read::Consumed);
                    note(&mut out, right, Read::Consumed);
                }
                Rvalue::FieldAccess { base, field } => {
                    let read = if field == "len" {
                        Read::Consumed
                    } else {
                        Read::Held
                    };
                    note(&mut out, base, read);
                }
                Rvalue::Call { func, args } => note_call(&mut out, func, args, retaining),
                Rvalue::Use(op)
                | Rvalue::UnaryOp(_, op)
                | Rvalue::Deref(op)
                | Rvalue::Len(op)
                | Rvalue::Cast { operand: op, .. }
                | Rvalue::EnumTag { operand: op, .. }
                | Rvalue::EnumPayload { operand: op, .. }
                | Rvalue::ClosureCreate { env: op, .. } => note(&mut out, op, Read::Held),
                Rvalue::Index { base, index } => {
                    note(&mut out, base, Read::Held);
                    note(&mut out, index, Read::Held);
                }
                Rvalue::AddressOf(place) => match place {
                    Place::Local(base) => out.push((*base, Read::Held)),
                    other => note_place(&mut out, other),
                },
                Rvalue::StructInit { fields, .. } => {
                    for (_, op) in fields {
                        note(&mut out, op, Read::Held);
                    }
                }
                Rvalue::EnumInit { payload, .. } => {
                    for op in payload {
                        note(&mut out, op, Read::Held);
                    }
                }
                Rvalue::SliceFromParts { ptr, len } => {
                    note(&mut out, ptr, Read::Held);
                    note(&mut out, len, Read::Held);
                }
            }
        }
        AirStmtKind::CallVoid { func, args } => note_call(&mut out, func, args, retaining),
        AirStmtKind::GcAlloc { local, .. }
        | AirStmtKind::Alloc { local, .. }
        | AirStmtKind::RcAlloc { local, .. }
        | AirStmtKind::GcDrop(local)
        | AirStmtKind::Free(local) => out.push((*local, Read::Held)),
        AirStmtKind::ArenaCreate(_)
        | AirStmtKind::ArenaDestroy(_)
        | AirStmtKind::MemoryFence(_) => {}
    }
    out
}

fn terminator_locals(terminator: &AirTerminator) -> Vec<LocalId> {
    let mut out = Vec::new();
    match terminator {
        AirTerminator::Return(Some(op)) => out.extend(operand_local(op)),
        AirTerminator::Branch { cond, .. } => out.extend(operand_local(cond)),
        AirTerminator::Switch { discr, .. } => out.extend(operand_local(discr)),
        AirTerminator::Invoke {
            func, args, ret, ..
        } => {
            if let Callee::FnPtr(id) = func {
                out.push(*id);
            }
            out.extend(args.iter().filter_map(operand_local));
            let mut held = Vec::new();
            note_place(&mut held, ret);
            out.extend(held.into_iter().map(|(id, _)| id));
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

fn produces_fresh_str(rvalue: &Rvalue, fresh_fn: &dyn Fn(&str) -> bool) -> bool {
    match rvalue {
        Rvalue::Call {
            func: Callee::FnPtr(_),
            ..
        } => true,
        Rvalue::Call {
            func: Callee::Named(name),
            ..
        } => STRING_PRODUCER_SYMBOLS.contains(&name.as_str()) || fresh_fn(name),
        Rvalue::BinaryOp(BinOp::Add, _, _) => true,
        // a literal took a share for every leaf as it was built
        Rvalue::StructInit { .. } | Rvalue::EnumInit { .. } => true,
        _ => false,
    }
}

fn is_unnamed_str_local(
    function: &AirFunction,
    carriers: &crate::counts::Carriers<'_>,
    target: LocalId,
) -> bool {
    function
        .locals
        .iter()
        .any(|local| local.id == target && local.name.is_none() && carriers.counted(&local.ty))
}

fn is_str_local(
    function: &AirFunction,
    carriers: &crate::counts::Carriers<'_>,
    target: LocalId,
) -> bool {
    function
        .locals
        .iter()
        .any(|local| local.id == target && carriers.counted(&local.ty))
}

fn is_to_string_call(rvalue: &Rvalue) -> bool {
    matches!(rvalue, Rvalue::Call { func: Callee::Named(name), .. } if name == TO_STRING)
}

pub(super) fn release_unbound_str_temps(
    function: &mut AirFunction,
    carriers: &crate::counts::Carriers<'_>,
    fresh_fns: &HashSet<String>,
    retaining: &HashSet<String>,
) {
    let mut defs: HashMap<LocalId, usize> = HashMap::new();
    let mut in_terminator: HashSet<LocalId> = HashSet::new();
    let mut reads: HashMap<LocalId, Vec<(usize, usize, Read)>> = HashMap::new();
    for (block_index, block) in function.blocks.iter().enumerate() {
        for (stmt_index, stmt) in block.stmts.iter().enumerate() {
            if let Some(dst) = assigned_local(&stmt.kind) {
                *defs.entry(dst).or_default() += 1;
            }
            for (id, read) in stmt_reads(&stmt.kind, retaining) {
                reads
                    .entry(id)
                    .or_default()
                    .push((block_index, stmt_index, read));
            }
        }
        if let AirTerminator::Invoke {
            ret: Place::Local(dst),
            ..
        } = &block.terminator
        {
            *defs.entry(*dst).or_default() += 1;
        }
        in_terminator.extend(terminator_locals(&block.terminator));
    }

    // an array literal is built by its element stores, so its last store defines it
    let mut built_in_place: HashMap<LocalId, (usize, usize)> = HashMap::new();
    for (block_index, block) in function.blocks.iter().enumerate() {
        for (stmt_index, stmt) in block.stmts.iter().enumerate() {
            if let AirStmtKind::Assign {
                place: Place::Index(base, _),
                ..
            } = &stmt.kind
            {
                built_in_place.insert(*base, (block_index, stmt_index));
            }
        }
    }

    let mut inserts: Vec<(usize, usize, LocalId)> = Vec::new();
    for (target, (block_index, def_index)) in built_in_place {
        if defs.contains_key(&target)
            || !is_unnamed_str_local(function, carriers, target)
            || in_terminator.contains(&target)
        {
            continue;
        }
        let own_store = |b: usize, s: usize| {
            matches!(&function.blocks[b].stmts[s].kind, AirStmtKind::Assign {
                place: Place::Index(base, _),
                ..
            } if *base == target)
        };
        let sites: Vec<(usize, usize, Read)> = reads
            .get(&target)
            .map(Vec::as_slice)
            .unwrap_or(&[])
            .iter()
            .filter(|(b, s, _)| !own_store(*b, *s))
            .copied()
            .collect();
        let all_consumed_after_def = sites
            .iter()
            .all(|(b, s, read)| *b == block_index && *s > def_index && *read != Read::Held);
        if !all_consumed_after_def {
            continue;
        }
        let last = sites.iter().map(|(_, s, _)| *s).max().unwrap_or(def_index);
        inserts.push((block_index, last + 1, target));
    }
    for (block_index, block) in function.blocks.iter().enumerate() {
        for (def_index, stmt) in block.stmts.iter().enumerate() {
            let AirStmtKind::Assign {
                place: Place::Local(target),
                rvalue,
            } = &stmt.kind
            else {
                continue;
            };
            if !produces_fresh_str(rvalue, &|name| fresh_fns.contains(name))
                || !is_unnamed_str_local(function, carriers, *target)
                || defs.get(target).copied() != Some(1)
                || in_terminator.contains(target)
            {
                continue;
            }
            let sites = reads.get(target).map(Vec::as_slice).unwrap_or(&[]);
            let all_consumed_after_def = sites
                .iter()
                .all(|(b, s, read)| *b == block_index && *s > def_index && *read != Read::Held);
            if !all_consumed_after_def {
                continue;
            }
            let printed_once = matches!(sites, [(_, _, Read::Printed)]);
            if printed_once && is_to_string_call(rvalue) {
                continue;
            }
            let last = sites.iter().map(|(_, s, _)| *s).max().unwrap_or(def_index);
            inserts.push((block_index, last + 1, *target));
        }
    }
    if inserts.is_empty() {
        return;
    }

    let mut next_local_id = function
        .locals
        .iter()
        .map(|local| local.id.0)
        .chain(function.params.iter().map(|param| param.id.0))
        .max()
        .map_or(0, |highest| highest + 1);
    inserts.sort_by(|a, b| (b.0, b.1).cmp(&(a.0, a.1)));
    for (block_index, at, target) in inserts {
        let addr = LocalId(next_local_id);
        next_local_id += 1;
        let slot = function
            .locals
            .iter()
            .find(|local| local.id == target)
            .map_or(AirType::Str, |local| local.ty.clone());
        let release = crate::counts::count_callee(&slot, false);
        function.locals.push(AirLocal {
            id: addr,
            ty: AirType::Ptr(Box::new(slot)),
            name: None,
            is_mut: false,
            span: None,
        });
        let block = &mut function.blocks[block_index];
        let span = block.stmts.get(at - 1).and_then(|s| s.span);
        block.stmts.insert(
            at,
            AirStmt {
                kind: AirStmtKind::CallVoid {
                    func: Callee::Named(release.to_string()),
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

pub(super) fn returns_only_fresh(
    function: &AirFunction,
    carriers: &crate::counts::Carriers<'_>,
    fresh_fn: &dyn Fn(&str) -> bool,
) -> bool {
    let mut defs: HashMap<LocalId, Vec<Option<&Rvalue>>> = HashMap::new();
    let mut stmt_read: HashSet<LocalId> = HashSet::new();
    let mut terminator_reads: HashMap<LocalId, usize> = HashMap::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                AirStmtKind::Assign {
                    place: Place::Local(dst),
                    rvalue,
                } => defs.entry(*dst).or_default().push(Some(rvalue)),
                kind => {
                    if let Some(dst) = assigned_local(kind) {
                        defs.entry(dst).or_default().push(None);
                    }
                }
            }
            stmt_read.extend(
                stmt_reads(&stmt.kind, &HashSet::new())
                    .into_iter()
                    .map(|(id, _)| id),
            );
        }
        if let AirTerminator::Invoke {
            ret: Place::Local(dst),
            ..
        } = &block.terminator
        {
            defs.entry(*dst).or_default().push(None);
        }
        for id in terminator_locals(&block.terminator) {
            *terminator_reads.entry(id).or_default() += 1;
        }
    }
    let reachable = crate::passes::validate::reachable_blocks(function);
    let mut live = function
        .blocks
        .iter()
        .filter(|block| reachable.contains(&block.id));
    live.all(|block| match &block.terminator {
        AirTerminator::Return(Some(Operand::Const(AirConst::Str(_)))) => true,
        AirTerminator::Return(Some(Operand::Move(r))) => is_str_local(function, carriers, *r),
        AirTerminator::Return(Some(Operand::Copy(t))) => {
            let made = defs.get(t).map(Vec::as_slice).unwrap_or(&[]);
            // an array is built by its element stores, each of which took its own share
            if made.is_empty() {
                return is_unnamed_str_local(function, carriers, *t)
                    && terminator_reads.get(t).copied() == Some(1);
            }
            let [Some(rvalue)] = made else {
                return false;
            };
            is_unnamed_str_local(function, carriers, *t)
                && produces_fresh_str(rvalue, fresh_fn)
                && !stmt_read.contains(t)
                && terminator_reads.get(t).copied() == Some(1)
        }
        AirTerminator::Return(_) => false,
        _ => true,
    })
}

pub fn fresh_returning_functions(
    functions: &[AirFunction],
    carriers: &crate::counts::Carriers<'_>,
    imported: &HashSet<String>,
) -> HashSet<String> {
    let mut proven: HashSet<String> = HashSet::new();
    loop {
        let mut grew = false;
        for function in functions {
            if function.is_extern
                || !carriers.counted(&function.ret_ty)
                || proven.contains(&function.name)
            {
                continue;
            }
            let fresh_fn = |name: &str| imported.contains(name) || proven.contains(name);
            if returns_only_fresh(function, carriers, &fresh_fn) {
                proven.insert(function.name.clone());
                grew = true;
            }
        }
        if !grew {
            return proven;
        }
    }
}

// every aelys body retains what it keeps, a generic one through the counts mono resolves
pub fn retaining_functions(functions: &[AirFunction]) -> HashSet<String> {
    functions
        .iter()
        .filter(|function| !function.is_extern)
        .map(|function| function.name.clone())
        .collect()
}

pub(super) fn retaining_declarations(program: &aelys_sema::TypedProgram) -> HashSet<String> {
    let mut decls: HashSet<String> = HashSet::new();
    crate::bir::build::for_each_fn_decl(&program.stmts, &mut |func, _| {
        if func.foreign.is_none() {
            decls.insert(func.name.clone());
        }
    });
    decls
}

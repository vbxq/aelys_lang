use std::collections::{HashMap, HashSet};

use crate::{
    AirFunction, AirStmt, AirStmtKind, AirTerminator, BlockId, Callee, LocalId, Operand, Place,
    Rvalue,
};

const STR_RETAIN: &str = "__aelys_str_retain";
const STR_RELEASE: &str = "__aelys_str_release";

fn local_of(op: &Operand) -> Option<LocalId> {
    match op {
        Operand::Copy(id) | Operand::Move(id) => Some(*id),
        Operand::Const(_) => None,
    }
}

fn named_call_arg(kind: &AirStmtKind, want: impl Fn(&str) -> bool) -> Option<LocalId> {
    let AirStmtKind::CallVoid {
        func: Callee::Named(name),
        args,
    } = kind
    else {
        return None;
    };
    if !want(name.as_str()) {
        return None;
    }
    args.first().and_then(local_of)
}

fn str_slot_call(kind: &AirStmtKind) -> Option<LocalId> {
    named_call_arg(kind, |name| name == STR_RETAIN || name == STR_RELEASE)
}

fn str_release_call(kind: &AirStmtKind) -> Option<LocalId> {
    named_call_arg(kind, |name| name == STR_RELEASE)
}

fn str_retain_call(kind: &AirStmtKind) -> Option<LocalId> {
    named_call_arg(kind, |name| name == STR_RETAIN)
}

fn assigned_local(kind: &AirStmtKind) -> Option<LocalId> {
    match kind {
        AirStmtKind::Assign {
            place: Place::Local(dst),
            ..
        } => Some(*dst),
        _ => None,
    }
}

fn overwritten_by_next(stmts: &[AirStmt], index: usize, base: LocalId) -> bool {
    stmts
        .get(index + 1)
        .and_then(|stmt| assigned_local(&stmt.kind))
        .is_some_and(|dst| dst == base)
}

fn successors(terminator: &AirTerminator) -> Vec<BlockId> {
    match terminator {
        AirTerminator::Goto(target) => vec![*target],
        AirTerminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        AirTerminator::Switch {
            targets, default, ..
        } => targets
            .iter()
            .map(|(_, block)| *block)
            .chain(std::iter::once(*default))
            .collect(),
        AirTerminator::Invoke { normal, unwind, .. } => vec![*normal, *unwind],
        AirTerminator::Return(_)
        | AirTerminator::Unwind
        | AirTerminator::Unreachable
        | AirTerminator::Panic { .. } => Vec::new(),
    }
}

// matching on the bare name is safe only because sema rejects a user redefinition of print/println with e0301
fn callee_stores_nothing(func: &Callee) -> bool {
    matches!(func, Callee::Named(name) if name == "print" || name == "println")
}

struct Scan {
    addr_of: HashMap<LocalId, LocalId>,
    slot_call_args: HashSet<LocalId>,
    escaped: HashSet<LocalId>,
}

impl Scan {
    fn escape(&mut self, op: &Operand) {
        if let Some(id) = local_of(op) {
            self.escaped.insert(id);
        }
    }

    fn escape_all(&mut self, ops: &[Operand]) {
        for op in ops {
            self.escape(op);
        }
    }

    fn escape_place_base(&mut self, place: &Place) {
        match place {
            Place::Local(_) | Place::Global(_) => {}
            Place::Field(base, _) | Place::Deref(base) => {
                self.escaped.insert(*base);
            }
            Place::Index(base, index) => {
                self.escaped.insert(*base);
                self.escape(index);
            }
        }
    }

    // every position is an escape unless it is named here, so a new air form fails closed
    fn note_rvalue(&mut self, dst: &Place, rvalue: &Rvalue) {
        match rvalue {
            // one scan and no fixpoint, so sparing a use here can delete a retain whose release stays
            Rvalue::Use(op) => self.escape(op),
            Rvalue::BinaryOp(_, _, _)
            | Rvalue::UnaryOp(_, _)
            // fieldaccess is safe only because sema keeps a string out of a field, so lifting e0714 makes it a uaf
            | Rvalue::FieldAccess { .. }
            | Rvalue::Deref(_)
            | Rvalue::Cast { .. }
            | Rvalue::Index { .. }
            | Rvalue::EnumTag { .. }
            | Rvalue::EnumPayload { .. }
            | Rvalue::Len(_) => {}
            Rvalue::AddressOf(place) => match place {
                Place::Local(base) => {
                    let reaches_only_the_slot_call =
                        matches!(dst, Place::Local(tmp) if self.slot_call_args.contains(tmp));
                    if !reaches_only_the_slot_call {
                        self.escaped.insert(*base);
                    }
                }
                other => self.escape_place_base(other),
            },
            Rvalue::Call { func, args } => {
                if !callee_stores_nothing(func) {
                    self.escape_all(args);
                }
            }
            Rvalue::StructInit { fields, .. } => {
                for (_, op) in fields {
                    self.escape(op);
                }
            }
            Rvalue::EnumInit { payload, .. } => self.escape_all(payload),
            Rvalue::ClosureCreate { env, .. } => self.escape(env),
            Rvalue::SliceFromParts { ptr, len } => {
                self.escape(ptr);
                self.escape(len);
            }
        }
    }

    fn note_stmt(&mut self, kind: &AirStmtKind) {
        match kind {
            AirStmtKind::Assign { place, rvalue } => {
                self.escape_place_base(place);
                self.note_rvalue(place, rvalue);
            }
            AirStmtKind::CallVoid { func, args } => {
                if str_slot_call(kind).is_none() && !callee_stores_nothing(func) {
                    self.escape_all(args);
                }
            }
            AirStmtKind::GcAlloc { local, .. }
            | AirStmtKind::Alloc { local, .. }
            | AirStmtKind::RcAlloc { local, .. }
            | AirStmtKind::GcDrop(local)
            | AirStmtKind::Free(local) => {
                self.escaped.insert(*local);
            }
            AirStmtKind::ArenaCreate(_)
            | AirStmtKind::ArenaDestroy(_)
            | AirStmtKind::MemoryFence(_) => {}
        }
    }

    fn note_terminator(&mut self, terminator: &AirTerminator) {
        match terminator {
            AirTerminator::Return(_)
            | AirTerminator::Goto(_)
            | AirTerminator::Branch { .. }
            | AirTerminator::Switch { .. }
            | AirTerminator::Unwind
            | AirTerminator::Unreachable
            | AirTerminator::Panic { .. } => {}
            AirTerminator::Invoke {
                args, ret, func, ..
            } => {
                if !callee_stores_nothing(func) {
                    self.escape_all(args);
                }
                self.escape_place_base(ret);
            }
        }
    }

    // the caller reads one statement's own escapes, so the set must start empty every time
    fn take_escapes(&mut self) -> Vec<LocalId> {
        let taken: Vec<LocalId> = self.escaped.iter().copied().collect();
        self.escaped.clear();
        taken
    }
}

pub(super) fn guard_escaping_str_locals(function: &mut AirFunction) {
    let mut scan = Scan {
        addr_of: HashMap::new(),
        slot_call_args: HashSet::new(),
        escaped: HashSet::new(),
    };
    let mut retained: HashSet<LocalId> = HashSet::new();

    for block in &function.blocks {
        for stmt in &block.stmts {
            if let AirStmtKind::Assign {
                place: Place::Local(dst),
                rvalue: Rvalue::AddressOf(Place::Local(base)),
            } = &stmt.kind
            {
                scan.addr_of.insert(*dst, *base);
            }
            if let Some(arg) = str_slot_call(&stmt.kind) {
                scan.slot_call_args.insert(arg);
            }
            if let Some(arg) = str_retain_call(&stmt.kind) {
                retained.insert(arg);
            }
        }
    }
    if scan.slot_call_args.is_empty() {
        return;
    }

    let mut escaped: HashSet<LocalId> = HashSet::new();
    let mut stmt_gen: Vec<Vec<Vec<LocalId>>> = Vec::with_capacity(function.blocks.len());
    let mut term_gen: Vec<Vec<LocalId>> = Vec::with_capacity(function.blocks.len());
    for block in &function.blocks {
        let mut escapes = Vec::with_capacity(block.stmts.len());
        for stmt in &block.stmts {
            scan.note_stmt(&stmt.kind);
            let taken = scan.take_escapes();
            escaped.extend(taken.iter().copied());
            escapes.push(taken);
        }
        scan.note_terminator(&block.terminator);
        let taken = scan.take_escapes();
        escaped.extend(taken.iter().copied());
        stmt_gen.push(escapes);
        term_gen.push(taken);
    }

    let index: HashMap<BlockId, usize> = function
        .blocks
        .iter()
        .enumerate()
        .map(|(i, block)| (block.id, i))
        .collect();
    let mut entry: Vec<HashSet<LocalId>> = vec![HashSet::new(); function.blocks.len()];
    loop {
        let mut changed = false;
        for (b, block) in function.blocks.iter().enumerate() {
            let mut live = entry[b].clone();
            for (i, stmt) in block.stmts.iter().enumerate() {
                live.extend(stmt_gen[b][i].iter().copied());
                if let Some(dst) = assigned_local(&stmt.kind) {
                    live.remove(&dst);
                }
            }
            live.extend(term_gen[b].iter().copied());
            for succ in successors(&block.terminator) {
                let Some(&s) = index.get(&succ) else {
                    continue;
                };
                for id in &live {
                    changed |= entry[s].insert(*id);
                }
            }
        }
        if !changed {
            break;
        }
    }

    let retained_bases: HashSet<LocalId> = retained
        .iter()
        .filter_map(|arg| scan.addr_of.get(arg).copied())
        .collect();

    let mut kept: HashSet<LocalId> = HashSet::new();
    for (b, block) in function.blocks.iter().enumerate() {
        let mut live = entry[b].clone();
        for (i, stmt) in block.stmts.iter().enumerate() {
            if let Some(arg) = str_release_call(&stmt.kind)
                && let Some(&base) = scan.addr_of.get(&arg)
                && !live.contains(&base)
                && !retained_bases.contains(&base)
                && overwritten_by_next(&block.stmts, i, base)
            {
                kept.insert(arg);
            }
            live.extend(stmt_gen[b][i].iter().copied());
            if let Some(dst) = assigned_local(&stmt.kind) {
                live.remove(&dst);
            }
        }
    }

    let dropped: HashSet<LocalId> = scan
        .slot_call_args
        .iter()
        .filter(|arg| !kept.contains(arg))
        .filter(|arg| {
            scan.addr_of
                .get(arg)
                .is_some_and(|base| escaped.contains(base))
        })
        .copied()
        .collect();
    if dropped.is_empty() {
        return;
    }
    for block in &mut function.blocks {
        block.stmts.retain(|stmt| {
            if let Some(arg) = str_slot_call(&stmt.kind) {
                return !dropped.contains(&arg);
            }
            if let AirStmtKind::Assign {
                place: Place::Local(dst),
                rvalue: Rvalue::AddressOf(Place::Local(_)),
            } = &stmt.kind
            {
                return !dropped.contains(dst);
            }
            true
        });
    }
}

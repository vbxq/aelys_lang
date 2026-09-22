use std::collections::{HashMap, HashSet};

use crate::{
    AirFunction, AirStmt, AirStmtKind, AirTerminator, AirType, BlockId, Callee, LocalId, Operand,
    Place, Rvalue, counts,
};

const VEC_POP: &str = "__aelys_vec_pop";

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
    named_call_arg(kind, |name| {
        counts::is_retain(name) || counts::is_release(name)
    })
}

fn str_release_call(kind: &AirStmtKind) -> Option<LocalId> {
    named_call_arg(kind, counts::is_release)
}

fn str_retain_call(kind: &AirStmtKind) -> Option<LocalId> {
    named_call_arg(kind, counts::is_retain)
}

// the runtime writes the popped element into this slot and keeps nothing
fn vec_pop_out_slot(kind: &AirStmtKind) -> Option<LocalId> {
    let AirStmtKind::CallVoid {
        func: Callee::Named(name),
        args,
    } = kind
    else {
        return None;
    };
    (name == VEC_POP)
        .then(|| args.get(1).and_then(local_of))
        .flatten()
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

// a retain reached before anything else reads the copy proves the copy holds its own share
fn retained_copy(stmts: &[AirStmt], i: usize, addr_of: &HashMap<LocalId, LocalId>) -> bool {
    let Some(AirStmtKind::Assign {
        place: Place::Local(dst),
        rvalue: Rvalue::Use(Operand::Copy(_) | Operand::Move(_)),
    }) = stmts.get(i).map(|s| &s.kind)
    else {
        return false;
    };
    let mut at = i + 1;
    while let Some(stmt) = stmts.get(at) {
        if let Some(addr) = assigned_local(&stmt.kind)
            && addr_of.get(&addr) == Some(dst)
        {
            return stmts.get(at + 1).and_then(|s| str_retain_call(&s.kind)) == Some(addr);
        }
        if !reads_only(&stmt.kind, *dst) {
            return false;
        }
        at += 1;
    }
    false
}

fn reads_only(kind: &AirStmtKind, dst: LocalId) -> bool {
    match kind {
        AirStmtKind::Assign {
            place: Place::Local(_),
            rvalue: Rvalue::FieldAccess { base, .. },
        } => local_of(base) == Some(dst),
        AirStmtKind::CallVoid {
            func: Callee::Named(name),
            ..
        } => name == "__aelys_rc_retain" || name == "__aelys_rc_release",
        _ => false,
    }
}

fn str_transfer(
    function: &AirFunction,
    carriers: &counts::Carriers<'_>,
    kind: &AirStmtKind,
) -> Option<(LocalId, LocalId)> {
    let AirStmtKind::Assign {
        place: Place::Local(dst),
        rvalue: Rvalue::Use(Operand::Move(src)),
    } = kind
    else {
        return None;
    };
    let is_str = |id: &LocalId| {
        function
            .locals
            .iter()
            .any(|l| l.id == *id && carriers.counted(&l.ty))
    };
    (is_str(dst) && is_str(src)).then_some((*dst, *src))
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

// matching on the bare name is safe only because sema rejects a user redefinition of print/println with E0301
fn callee_stores_nothing(func: &Callee) -> bool {
    matches!(func, Callee::Named(name) if name == "print" || name == "println" || name == VEC_POP
        || crate::symbols::STRING_READER_SYMBOLS.contains(&name.as_str()))
}

struct Scan<'r> {
    addr_of: HashMap<LocalId, LocalId>,
    slot_call_args: HashSet<LocalId>,
    lent: HashSet<LocalId>,
    escaped: HashSet<LocalId>,
    retaining: &'r HashSet<String>,
}

impl Scan<'_> {
    // a function value is never external (E0616), so its body retains what it keeps
    fn callee_keeps_no_borrow(&self, func: &Callee) -> bool {
        callee_stores_nothing(func)
            || matches!(func, Callee::FnPtr(_))
            || matches!(func, Callee::Named(name) if self.retaining.contains(name))
    }

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

    // a store into a field or an element writes the local, it does not hand it to anyone
    fn escape_place_base(&mut self, place: &Place) {
        match place {
            Place::Local(_) | Place::Global(_) | Place::Field(_, _) | Place::Deref(_) => {}
            Place::Index(_, index) => self.escape(index),
        }
    }

    // every position is an escape unless it is named here, so a new air form fails closed
    fn note_rvalue(&mut self, dst: &Place, rvalue: &Rvalue) {
        match rvalue {
            Rvalue::Use(op) => self.escape(op),
            Rvalue::BinaryOp(_, _, _)
            | Rvalue::UnaryOp(_, _)
            // fieldaccess is safe only because sema keeps a string out of a field, so lifting E0714 makes it a uaf
            | Rvalue::FieldAccess { .. }
            | Rvalue::Deref(_)
            | Rvalue::Cast { .. }
            | Rvalue::Index { .. }
            | Rvalue::EnumTag { .. }
            | Rvalue::EnumPayload { .. }
            | Rvalue::Len(_) => {}
            Rvalue::AddressOf(place) => match place {
                Place::Local(base) => {
                    let reaches_only_the_slot_call = matches!(dst, Place::Local(tmp)
                        if self.slot_call_args.contains(tmp) || self.lent.contains(tmp));
                    if !reaches_only_the_slot_call {
                        self.escaped.insert(*base);
                    }
                }
                other => self.escape_place_base(other),
            },
            Rvalue::Call { func, args } => {
                if !self.callee_keeps_no_borrow(func) {
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
                if str_slot_call(kind).is_none() && !self.callee_keeps_no_borrow(func) {
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
                if !self.callee_keeps_no_borrow(func) {
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

struct Escapes<'r> {
    scan: Scan<'r>,
    transfers: Vec<(usize, usize, LocalId, LocalId)>,
    retained: HashSet<LocalId>,
    released: HashSet<LocalId>,
    escaped: HashSet<LocalId>,
    stmt_gen: Vec<Vec<Vec<LocalId>>>,
    term_gen: Vec<Vec<LocalId>>,
}

fn scan_escapes<'r>(
    function: &AirFunction,
    carriers: &counts::Carriers<'_>,
    retaining: &'r HashSet<String>,
) -> Escapes<'r> {
    let mut scan = Scan {
        addr_of: HashMap::new(),
        slot_call_args: HashSet::new(),
        lent: HashSet::new(),
        escaped: HashSet::new(),
        retaining,
    };
    let mut retained: HashSet<LocalId> = HashSet::new();
    let mut released: HashSet<LocalId> = HashSet::new();

    for block in &function.blocks {
        for stmt in &block.stmts {
            if let AirStmtKind::Assign {
                place: Place::Local(dst),
                rvalue: Rvalue::AddressOf(Place::Local(base)),
            } = &stmt.kind
            {
                scan.addr_of.insert(*dst, *base);
            }
            if let Some(arg) = str_slot_call(&stmt.kind).or_else(|| vec_pop_out_slot(&stmt.kind)) {
                scan.slot_call_args.insert(arg);
            }
            if let Some(arg) = str_retain_call(&stmt.kind) {
                retained.insert(arg);
            }
            if let Some(arg) = str_release_call(&stmt.kind) {
                released.insert(arg);
            }
        }
    }

    scan.lent = lent_addresses(function, &scan);

    let mut escaped: HashSet<LocalId> = HashSet::new();
    let mut transfers = Vec::new();
    let mut stmt_gen: Vec<Vec<Vec<LocalId>>> = Vec::with_capacity(function.blocks.len());
    let mut term_gen: Vec<Vec<LocalId>> = Vec::with_capacity(function.blocks.len());
    for (b, block) in function.blocks.iter().enumerate() {
        let mut escapes = Vec::with_capacity(block.stmts.len());
        for (i, stmt) in block.stmts.iter().enumerate() {
            if retained_copy(&block.stmts, i, &scan.addr_of) {
                escapes.push(Vec::new());
                continue;
            }
            if let Some((dst, src)) = str_transfer(function, carriers, &stmt.kind) {
                transfers.push((b, i, dst, src));
                escapes.push(Vec::new());
                continue;
            }
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
    Escapes {
        scan,
        transfers,
        retained,
        released,
        escaped,
        stmt_gen,
        term_gen,
    }
}

// an address kept for place bases and retaining callees cannot outlive its slot, per E0714 and E0722
fn lent_addresses(function: &AirFunction, scan: &Scan<'_>) -> HashSet<LocalId> {
    let mut root: HashMap<LocalId, LocalId> = scan.addr_of.keys().map(|a| (*a, *a)).collect();
    loop {
        let mut grew = false;
        for stmt in function.blocks.iter().flat_map(|b| &b.stmts) {
            let AirStmtKind::Assign {
                place: Place::Local(dst),
                rvalue,
            } = &stmt.kind
            else {
                continue;
            };
            let src = match rvalue {
                Rvalue::Use(Operand::Copy(p) | Operand::Move(p))
                | Rvalue::AddressOf(Place::Field(p, _) | Place::Index(p, _) | Place::Deref(p)) => {
                    Some(*p)
                }
                _ => None,
            };
            if let Some(r) = src.and_then(|p| root.get(&p).copied())
                && !root.contains_key(dst)
            {
                root.insert(*dst, r);
                grew = true;
            }
        }
        if !grew {
            break;
        }
    }
    let mut bad: HashSet<LocalId> = HashSet::new();
    let mut mark = |op: &Operand| {
        if let Some(r) = local_of(op).and_then(|p| root.get(&p)) {
            bad.insert(*r);
        }
    };
    for block in &function.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                AirStmtKind::Assign { place, rvalue } => {
                    if let Place::Index(_, index) = place {
                        mark(index);
                    }
                    match rvalue {
                        Rvalue::Use(op) => {
                            if !matches!(place, Place::Local(d) if root.contains_key(d)) {
                                mark(op);
                            }
                        }
                        Rvalue::FieldAccess { .. } | Rvalue::Deref(_) | Rvalue::Len(_) => {}
                        Rvalue::Index { index, .. } => mark(index),
                        Rvalue::AddressOf(place) => {
                            if let Place::Index(_, index) = place {
                                mark(index);
                            }
                        }
                        Rvalue::Call { func, args } => {
                            if !scan.callee_keeps_no_borrow(func) {
                                args.iter().for_each(&mut mark);
                            }
                        }
                        Rvalue::BinaryOp(_, left, right) => {
                            mark(left);
                            mark(right);
                        }
                        Rvalue::UnaryOp(_, op)
                        | Rvalue::Cast { operand: op, .. }
                        | Rvalue::EnumTag { operand: op, .. }
                        | Rvalue::EnumPayload { operand: op, .. }
                        | Rvalue::ClosureCreate { env: op, .. } => mark(op),
                        Rvalue::StructInit { fields, .. } => {
                            fields.iter().for_each(|(_, op)| mark(op));
                        }
                        Rvalue::EnumInit { payload, .. } => payload.iter().for_each(&mut mark),
                        Rvalue::SliceFromParts { ptr, len } => {
                            mark(ptr);
                            mark(len);
                        }
                    }
                }
                AirStmtKind::CallVoid { func, args } => {
                    if str_slot_call(&stmt.kind).is_none() && !scan.callee_keeps_no_borrow(func) {
                        args.iter().for_each(&mut mark);
                    }
                }
                AirStmtKind::GcAlloc { local, .. }
                | AirStmtKind::Alloc { local, .. }
                | AirStmtKind::RcAlloc { local, .. }
                | AirStmtKind::GcDrop(local)
                | AirStmtKind::Free(local) => mark(&Operand::Copy(*local)),
                AirStmtKind::ArenaCreate(_)
                | AirStmtKind::ArenaDestroy(_)
                | AirStmtKind::MemoryFence(_) => {}
            }
        }
        match &block.terminator {
            AirTerminator::Return(Some(op)) => mark(op),
            AirTerminator::Branch { cond, .. } => mark(cond),
            AirTerminator::Switch { discr, .. } => mark(discr),
            AirTerminator::Invoke { func, args, .. } => {
                if !scan.callee_keeps_no_borrow(func) {
                    args.iter().for_each(&mut mark);
                }
            }
            AirTerminator::Return(None)
            | AirTerminator::Goto(_)
            | AirTerminator::Unwind
            | AirTerminator::Unreachable
            | AirTerminator::Panic { .. } => {}
        }
    }
    scan.addr_of
        .keys()
        .filter(|a| !bad.contains(a))
        .copied()
        .collect()
}

pub(super) fn guard_escaping_str_locals(
    function: &mut AirFunction,
    carriers: &counts::Carriers<'_>,
    retaining: &HashSet<String>,
) {
    let Escapes {
        scan,
        transfers,
        retained,
        released,
        escaped,
        stmt_gen,
        term_gen,
    } = scan_escapes(function, carriers, retaining);
    drop_escaped_releases(
        function, &scan, &retained, &released, &escaped, &stmt_gen, &term_gen,
    );
    retain_escaped_returns(function, carriers, &escaped);
    retain_escaped_transfers(function, carriers, &transfers, &escaped);
}

fn drop_escaped_releases(
    function: &mut AirFunction,
    scan: &Scan<'_>,
    retained: &HashSet<LocalId>,
    released: &HashSet<LocalId>,
    escaped: &HashSet<LocalId>,
    stmt_gen: &[Vec<Vec<LocalId>>],
    term_gen: &[Vec<LocalId>],
) {
    if scan.slot_call_args.is_empty() {
        return;
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

    // retains are never dropped: an escaping local leaves its share with its alias, a leak rather than a free
    let dropped: HashSet<LocalId> = released
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

fn next_free_local(function: &AirFunction) -> u32 {
    function
        .locals
        .iter()
        .map(|local| local.id.0)
        .chain(function.params.iter().map(|param| param.id.0))
        .max()
        .map_or(0, |highest| highest + 1)
}

fn retain_stmts(function: &mut AirFunction, target: LocalId, next: &mut u32) -> [AirStmt; 2] {
    let addr = LocalId(*next);
    *next += 1;
    let slot = function
        .locals
        .iter()
        .find(|l| l.id == target)
        .map_or(AirType::Str, |l| l.ty.clone());
    let retain = counts::count_callee(&slot, true);
    function.locals.push(crate::AirLocal {
        id: addr,
        ty: AirType::Ptr(Box::new(slot)),
        name: None,
        is_mut: false,
        span: None,
    });
    [
        AirStmt {
            kind: AirStmtKind::Assign {
                place: Place::Local(addr),
                rvalue: Rvalue::AddressOf(Place::Local(target)),
            },
            span: None,
        },
        AirStmt {
            kind: AirStmtKind::CallVoid {
                func: Callee::Named(retain.to_string()),
                args: vec![Operand::Copy(addr)],
            },
            span: None,
        },
    ]
}

fn retain_escaped_transfers(
    function: &mut AirFunction,
    carriers: &counts::Carriers<'_>,
    transfers: &[(usize, usize, LocalId, LocalId)],
    escaped: &HashSet<LocalId>,
) {
    let due: Vec<(usize, LocalId, LocalId)> = transfers
        .iter()
        .filter(|(_, _, _, src)| escaped.contains(src))
        .map(|(b, _, dst, src)| (*b, *dst, *src))
        .collect();
    if due.is_empty() {
        return;
    }
    let mut next = next_free_local(function);
    for (b, dst, src) in due {
        let Some(at) = function.blocks[b]
            .stmts
            .iter()
            .position(|stmt| str_transfer(function, carriers, &stmt.kind) == Some((dst, src)))
        else {
            continue;
        };
        let stmts = retain_stmts(function, dst, &mut next);
        let block = &mut function.blocks[b];
        for (k, stmt) in stmts.into_iter().enumerate() {
            block.stmts.insert(at + 1 + k, stmt);
        }
    }
}

fn retain_escaped_returns(
    function: &mut AirFunction,
    carriers: &counts::Carriers<'_>,
    escaped: &HashSet<LocalId>,
) {
    let returned: Vec<(usize, LocalId)> = function
        .blocks
        .iter()
        .enumerate()
        .filter_map(|(b, block)| match &block.terminator {
            AirTerminator::Return(Some(Operand::Move(r))) if escaped.contains(r) => Some((b, *r)),
            _ => None,
        })
        .filter(|(_, r)| {
            function
                .locals
                .iter()
                .any(|l| l.id == *r && carriers.counted(&l.ty))
        })
        .collect();
    if returned.is_empty() {
        return;
    }
    let mut next = next_free_local(function);
    for (b, r) in returned {
        let stmts = retain_stmts(function, r, &mut next);
        function.blocks[b].stmts.extend(stmts);
    }
}

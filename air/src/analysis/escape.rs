use crate::{AirFunction, AirStmtKind, Callee, LocalId, Operand, Place, Rvalue};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EscapeKind {
    Return,
    StoreIntoAggregate,
    Call,
    Closure,
    AddressOf,
}

pub fn escapes(function: &AirFunction, local: LocalId) -> Option<EscapeKind> {
    for block in &function.blocks {
        for stmt in &block.stmts {
            if let Some(k) = stmt_escape(&stmt.kind, local) {
                return Some(k);
            }
        }
        if let Some(k) = term_escape(&block.terminator, local) {
            return Some(k);
        }
    }
    None
}

fn stmt_escape(kind: &AirStmtKind, local: LocalId) -> Option<EscapeKind> {
    match kind {
        AirStmtKind::Assign { place, rvalue } => {
            if place_is_aggregate(place) && rvalue_has_operand(rvalue, local) {
                return Some(EscapeKind::StoreIntoAggregate);
            }
            match rvalue {
                Rvalue::StructInit { fields, .. } => {
                    if fields.iter().any(|(_, op)| operand_is(op, local)) {
                        return Some(EscapeKind::StoreIntoAggregate);
                    }
                }
                Rvalue::EnumInit { payload, .. } => {
                    if payload.iter().any(|op| operand_is(op, local)) {
                        return Some(EscapeKind::StoreIntoAggregate);
                    }
                }
                Rvalue::ClosureCreate { env, .. } => {
                    if operand_is(env, local) {
                        return Some(EscapeKind::Closure);
                    }
                }
                Rvalue::Call { func, args } => {
                    if callee_is(func, local) || args.iter().any(|op| operand_is(op, local)) {
                        return Some(EscapeKind::Call);
                    }
                }
                // pointer local marks that pointer; when the pointer is a parameter the storage
                // it names belongs to the caller and has no local here to mark
                Rvalue::AddressOf(p) if place_base(p) == Some(local) => {
                    return Some(EscapeKind::AddressOf);
                }
                _ => {}
            }
            None
        }
        AirStmtKind::CallVoid { func, args } => {
            if is_own_rc_bookkeeping(func, args, local) {
                return None;
            }
            if callee_is(func, local) || args.iter().any(|op| operand_is(op, local)) {
                return Some(EscapeKind::Call);
            }
            None
        }
        _ => None,
    }
}

fn term_escape(term: &crate::AirTerminator, local: LocalId) -> Option<EscapeKind> {
    match term {
        crate::AirTerminator::Return(Some(op)) if operand_is(op, local) => Some(EscapeKind::Return),
        crate::AirTerminator::Invoke {
            func, args, ret, ..
        } => {
            if callee_is(func, local)
                || args.iter().any(|op| operand_is(op, local))
                || (place_is_aggregate(ret) && place_base(ret) == Some(local))
            {
                Some(EscapeKind::Call)
            } else {
                None
            }
        }
        _ => None,
    }
}

// keyed on the rvalue operand, not the place base: `b.next = a` must flag `a`, not `b`
pub fn stored_into_aggregate_set(function: &AirFunction) -> HashSet<LocalId> {
    let mut set = HashSet::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            if let AirStmtKind::Assign { place, rvalue } = &stmt.kind {
                if place_is_aggregate(place) {
                    for_each_rvalue_operand_local(rvalue, |l| {
                        set.insert(l);
                    });
                }
                // construction payloads escape into the aggregate whatever the place is
                match rvalue {
                    Rvalue::StructInit { fields, .. } => {
                        for (_, op) in fields {
                            if let Operand::Copy(l) | Operand::Move(l) = op {
                                set.insert(*l);
                            }
                        }
                    }
                    Rvalue::EnumInit { payload, .. } => {
                        for op in payload {
                            if let Operand::Copy(l) | Operand::Move(l) = op {
                                set.insert(*l);
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // invoke args are call-escapes, not aggregate stores, so terminators add nothing
        let _ = &block.terminator;
    }
    set
}

// stored operand escapes exactly as a field store does
fn place_is_aggregate(place: &Place) -> bool {
    matches!(
        place,
        Place::Field(_, _) | Place::Index(_, _) | Place::Deref(_) | Place::Global(_)
    )
}

fn place_base(place: &Place) -> Option<LocalId> {
    match place {
        Place::Local(l) | Place::Field(l, _) | Place::Deref(l) | Place::Index(l, _) => Some(*l),
        Place::Global(_) => None,
    }
}

fn operand_is(op: &Operand, local: LocalId) -> bool {
    matches!(op, Operand::Copy(l) | Operand::Move(l) if *l == local)
}

fn callee_is(callee: &Callee, local: LocalId) -> bool {
    matches!(callee, Callee::FnPtr(l) if *l == local)
}

fn rvalue_has_operand(rvalue: &Rvalue, local: LocalId) -> bool {
    let mut found = false;
    for_each_rvalue_operand_local(rvalue, |l| {
        if l == local {
            found = true;
        }
    });
    found
}

fn for_each_rvalue_operand_local(rvalue: &Rvalue, mut f: impl FnMut(LocalId)) {
    match rvalue {
        Rvalue::Use(op) | Rvalue::UnaryOp(_, op) | Rvalue::Deref(op) => operand_local(op, &mut f),
        Rvalue::BinaryOp(_, a, b) => {
            operand_local(a, &mut f);
            operand_local(b, &mut f);
        }
        Rvalue::Call { func, args } => {
            if let Callee::FnPtr(l) = func {
                f(*l);
            }
            for a in args {
                operand_local(a, &mut f);
            }
        }
        Rvalue::StructInit { fields, .. } => {
            for (_, op) in fields {
                operand_local(op, &mut f);
            }
        }
        Rvalue::FieldAccess { base, .. } | Rvalue::Cast { operand: base, .. } => {
            operand_local(base, &mut f)
        }
        Rvalue::Index { base, index } => {
            operand_local(base, &mut f);
            operand_local(index, &mut f);
        }
        Rvalue::AddressOf(p) => {
            if let Some(l) = place_base(p) {
                f(l);
            }
            if let Place::Index(_, idx) = p {
                operand_local(idx, &mut f);
            }
        }
        Rvalue::EnumInit { payload, .. } => {
            for op in payload {
                operand_local(op, &mut f);
            }
        }
        Rvalue::EnumTag { operand, .. } => operand_local(operand, &mut f),
        Rvalue::EnumPayload { operand, .. } => operand_local(operand, &mut f),
        Rvalue::ClosureCreate { env, .. } => operand_local(env, &mut f),
        Rvalue::SliceFromParts { ptr, len } => {
            operand_local(ptr, &mut f);
            operand_local(len, &mut f);
        }
    }
}

fn operand_local(op: &Operand, f: &mut impl FnMut(LocalId)) {
    if let Operand::Copy(l) | Operand::Move(l) = op {
        f(*l);
    }
}

// a local's own retain/release is refcount bookkeeping, not a hand-off to another owner
fn is_own_rc_bookkeeping(func: &Callee, args: &[Operand], local: LocalId) -> bool {
    if let Callee::Named(name) = func {
        if (name == "__aelys_rc_retain" || name == "__aelys_rc_release")
            && args.len() == 1
            && operand_is(&args[0], local)
        {
            return true;
        }
    }
    false
}

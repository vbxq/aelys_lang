use crate::{
    AirBlock, AirFunction, AirStmtKind, AirTerminator, BlockId, Callee, LocalId, Operand, Place,
    Rvalue,
};
use std::collections::HashMap;

pub fn successors(term: &AirTerminator) -> Vec<BlockId> {
    match term {
        AirTerminator::Goto(target) => vec![*target],
        AirTerminator::Branch {
            then_block,
            else_block,
            ..
        } => vec![*then_block, *else_block],
        AirTerminator::Switch {
            targets, default, ..
        } => {
            let mut out: Vec<BlockId> = targets.iter().map(|(_, b)| *b).collect();
            out.push(*default);
            out
        }
        AirTerminator::Invoke { normal, unwind, .. } => vec![*normal, *unwind],
        AirTerminator::Return(_)
        | AirTerminator::Unwind
        | AirTerminator::Unreachable
        | AirTerminator::Panic { .. } => Vec::new(),
    }
}

// blocks are not guaranteed to be index-keyed by id value, so callers need this map
pub fn block_index_map(function: &AirFunction) -> HashMap<BlockId, usize> {
    function
        .blocks
        .iter()
        .enumerate()
        .map(|(i, b)| (b.id, i))
        .collect()
}

pub fn predecessors(function: &AirFunction) -> HashMap<BlockId, Vec<BlockId>> {
    let mut preds: HashMap<BlockId, Vec<BlockId>> = HashMap::new();
    for b in &function.blocks {
        preds.entry(b.id).or_default();
    }
    for b in &function.blocks {
        for s in successors(&b.terminator) {
            preds.entry(s).or_default().push(b.id);
        }
    }
    preds
}

pub fn for_each_use_stmt(kind: &AirStmtKind, mut f: impl FnMut(LocalId)) {
    visit_stmt_uses(kind, &mut f);
}

pub fn for_each_use_term(term: &AirTerminator, mut f: impl FnMut(LocalId)) {
    visit_term_uses(term, &mut f);
}

fn visit_stmt_uses(kind: &AirStmtKind, f: &mut impl FnMut(LocalId)) {
    match kind {
        AirStmtKind::Assign { place, rvalue } => {
            visit_place_uses(place, f);
            visit_rvalue_uses(rvalue, f);
        }
        // these define the local, so they are defs and not uses (dead_locals differs here)
        AirStmtKind::GcAlloc { .. }
        | AirStmtKind::Alloc { .. }
        | AirStmtKind::RcAlloc { .. } => {}
        AirStmtKind::GcDrop(local) | AirStmtKind::Free(local) => f(*local),
        AirStmtKind::CallVoid { func, args } => {
            visit_callee_uses(func, f);
            for arg in args {
                visit_operand_uses(arg, f);
            }
        }
        AirStmtKind::ArenaCreate(_)
        | AirStmtKind::ArenaDestroy(_)
        | AirStmtKind::MemoryFence(_) => {}
    }
}

fn visit_term_uses(term: &AirTerminator, f: &mut impl FnMut(LocalId)) {
    match term {
        AirTerminator::Return(Some(op)) => visit_operand_uses(op, f),
        AirTerminator::Branch { cond, .. } => visit_operand_uses(cond, f),
        AirTerminator::Switch { discr, .. } => visit_operand_uses(discr, f),
        AirTerminator::Invoke {
            func, args, ret, ..
        } => {
            visit_callee_uses(func, f);
            for arg in args {
                visit_operand_uses(arg, f);
            }
            visit_place_uses(ret, f);
        }
        AirTerminator::Return(None)
        | AirTerminator::Goto(_)
        | AirTerminator::Unwind
        | AirTerminator::Unreachable
        | AirTerminator::Panic { .. } => {}
    }
}

fn visit_rvalue_uses(rvalue: &Rvalue, f: &mut impl FnMut(LocalId)) {
    match rvalue {
        Rvalue::Use(op) | Rvalue::UnaryOp(_, op) | Rvalue::Deref(op) => visit_operand_uses(op, f),
        Rvalue::BinaryOp(_, left, right) => {
            visit_operand_uses(left, f);
            visit_operand_uses(right, f);
        }
        Rvalue::Call { func, args } => {
            visit_callee_uses(func, f);
            for arg in args {
                visit_operand_uses(arg, f);
            }
        }
        Rvalue::StructInit { fields, .. } => {
            for (_, operand) in fields {
                visit_operand_uses(operand, f);
            }
        }
        Rvalue::FieldAccess { base, .. } | Rvalue::Cast { operand: base, .. } => {
            visit_operand_uses(base, f);
        }
        Rvalue::Index { base, index } => {
            visit_operand_uses(base, f);
            visit_operand_uses(index, f);
        }
        Rvalue::AddressOf(local) => f(*local),
        Rvalue::EnumInit { payload, .. } => {
            for operand in payload {
                visit_operand_uses(operand, f);
            }
        }
        Rvalue::EnumTag { operand, .. } => visit_operand_uses(operand, f),
        Rvalue::EnumPayload { operand, .. } => visit_operand_uses(operand, f),
        Rvalue::ClosureCreate { env, .. } => visit_operand_uses(env, f),
    }
}

fn visit_place_uses(place: &Place, f: &mut impl FnMut(LocalId)) {
    match place {
        // a write-only destination is a def, not a use
        Place::Local(_) => {}
        Place::Field(local, _) | Place::Deref(local) => f(*local),
        Place::Index(local, operand) => {
            f(*local);
            visit_operand_uses(operand, f);
        }
    }
}

fn visit_operand_uses(operand: &Operand, f: &mut impl FnMut(LocalId)) {
    match operand {
        Operand::Copy(local) | Operand::Move(local) => f(*local),
        Operand::Const(_) => {}
    }
}

fn visit_callee_uses(callee: &Callee, f: &mut impl FnMut(LocalId)) {
    if let Callee::FnPtr(local) = callee {
        f(*local);
    }
}

pub fn block_uses(block: &AirBlock, mut f: impl FnMut(LocalId)) {
    for stmt in &block.stmts {
        visit_stmt_uses(&stmt.kind, &mut f);
    }
    visit_term_uses(&block.terminator, &mut f);
}

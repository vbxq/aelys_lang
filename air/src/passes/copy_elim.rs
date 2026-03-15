use crate::{
    AirFunction, AirProgram, AirStmt, AirStmtKind, AirTerminator, Callee, LocalId, Operand, Place,
    Rvalue,
};
use std::collections::{HashMap, HashSet};

pub fn eliminate_copies(program: &mut AirProgram) {
    for function in &mut program.functions {
        eliminate_function_copies(function);
    }
}

fn eliminate_function_copies(function: &mut AirFunction) {
    let params: HashSet<LocalId> = function.params.iter().map(|p| p.id).collect();
    if params.is_empty() {
        return;
    }

    let writes = collect_write_counts(function);
    let direct_aliases = collect_direct_aliases(function, &writes);
    let replacements = resolve_to_params(&direct_aliases, &params);
    if replacements.is_empty() {
        return;
    }

    for block in &mut function.blocks {
        block
            .stmts
            .retain(|stmt| !is_eliminated_copy_stmt(stmt, &replacements));
        for stmt in &mut block.stmts {
            rewrite_stmt(stmt, &replacements);
        }
        rewrite_terminator(&mut block.terminator, &replacements);
    }
}

fn collect_write_counts(function: &AirFunction) -> HashMap<LocalId, u32> {
    let mut counts = HashMap::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                AirStmtKind::Assign { place, .. } => bump_place_write(place, &mut counts),
                AirStmtKind::GcAlloc { local, .. } | AirStmtKind::Alloc { local, .. } => {
                    bump_local(*local, &mut counts)
                }
                _ => {}
            }
        }

        if let AirTerminator::Invoke { ret, .. } = &block.terminator {
            bump_place_write(ret, &mut counts);
        }
    }
    counts
}

fn collect_direct_aliases(
    function: &AirFunction,
    writes: &HashMap<LocalId, u32>,
) -> HashMap<LocalId, LocalId> {
    let mut aliases = HashMap::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            let (dst, src) = match copy_stmt_locals(stmt) {
                Some(pair) => pair,
                None => continue,
            };
            if dst == src {
                continue;
            }
            // Only safe to alias when dst is written exactly once (the copy itself)
            // AND src is never written in the body (it's immutable). If src is
            // modified later (e.g. a param reassigned in a loop), the alias would
            // replace dst with a stale/wrong value.
            if writes.get(&dst).copied().unwrap_or(0) == 1
                && writes.get(&src).copied().unwrap_or(0) == 0
            {
                aliases.insert(dst, src);
            }
        }
    }
    aliases
}

fn resolve_to_params(
    direct_aliases: &HashMap<LocalId, LocalId>,
    params: &HashSet<LocalId>,
) -> HashMap<LocalId, LocalId> {
    let mut replacements = HashMap::new();
    for &dst in direct_aliases.keys() {
        if let Some(param) = resolve_param_target(dst, direct_aliases, params) {
            replacements.insert(dst, param);
        }
    }
    replacements
}

fn resolve_param_target(
    dst: LocalId,
    direct_aliases: &HashMap<LocalId, LocalId>,
    params: &HashSet<LocalId>,
) -> Option<LocalId> {
    let mut seen = HashSet::new();
    let mut current = *direct_aliases.get(&dst)?;
    while !params.contains(&current) {
        if !seen.insert(current) {
            return None;
        }
        current = *direct_aliases.get(&current)?;
    }
    Some(current)
}

fn is_eliminated_copy_stmt(stmt: &AirStmt, replacements: &HashMap<LocalId, LocalId>) -> bool {
    let (dst, src) = match copy_stmt_locals(stmt) {
        Some(pair) => pair,
        None => return false,
    };
    match replacements.get(&dst).copied() {
        Some(target) => rewrite_local(src, replacements) == target,
        None => false,
    }
}

fn copy_stmt_locals(stmt: &AirStmt) -> Option<(LocalId, LocalId)> {
    match &stmt.kind {
        AirStmtKind::Assign {
            place: Place::Local(dst),
            rvalue: Rvalue::Use(Operand::Copy(src) | Operand::Move(src)),
        } => Some((*dst, *src)),
        _ => None,
    }
}

fn rewrite_stmt(stmt: &mut AirStmt, replacements: &HashMap<LocalId, LocalId>) {
    match &mut stmt.kind {
        AirStmtKind::Assign { place, rvalue } => {
            rewrite_place(place, replacements);
            rewrite_rvalue(rvalue, replacements);
        }
        AirStmtKind::CallVoid { func, args } => {
            rewrite_callee(func, replacements);
            for arg in args {
                rewrite_operand(arg, replacements);
            }
        }
        AirStmtKind::GcDrop(local) | AirStmtKind::Free(local) => {
            *local = rewrite_local(*local, replacements);
        }
        _ => {}
    }
}

fn rewrite_terminator(term: &mut AirTerminator, replacements: &HashMap<LocalId, LocalId>) {
    match term {
        AirTerminator::Return(Some(op)) => rewrite_operand(op, replacements),
        AirTerminator::Branch { cond, .. } => rewrite_operand(cond, replacements),
        AirTerminator::Switch { discr, .. } => rewrite_operand(discr, replacements),
        AirTerminator::Invoke {
            func, args, ret, ..
        } => {
            rewrite_callee(func, replacements);
            for arg in args {
                rewrite_operand(arg, replacements);
            }
            rewrite_place(ret, replacements);
        }
        _ => {}
    }
}

fn rewrite_rvalue(value: &mut Rvalue, replacements: &HashMap<LocalId, LocalId>) {
    match value {
        Rvalue::Use(op) | Rvalue::UnaryOp(_, op) | Rvalue::Deref(op) => {
            rewrite_operand(op, replacements);
        }
        Rvalue::BinaryOp(_, left, right) => {
            rewrite_operand(left, replacements);
            rewrite_operand(right, replacements);
        }
        Rvalue::Call { func, args } => {
            rewrite_callee(func, replacements);
            for arg in args {
                rewrite_operand(arg, replacements);
            }
        }
        Rvalue::StructInit { fields, .. } => {
            for (_, operand) in fields {
                rewrite_operand(operand, replacements);
            }
        }
        Rvalue::FieldAccess { base, .. } | Rvalue::Cast { operand: base, .. } => {
            rewrite_operand(base, replacements);
        }
        Rvalue::Index { base, index } => {
            rewrite_operand(base, replacements);
            rewrite_operand(index, replacements);
        }
        Rvalue::AddressOf(local) => {
            *local = rewrite_local(*local, replacements);
        }
        Rvalue::EnumInit { payload, .. } => {
            for operand in payload {
                rewrite_operand(operand, replacements);
            }
        }
        Rvalue::EnumTag { operand, .. } => {
            rewrite_operand(operand, replacements);
        }
        Rvalue::EnumPayload { operand, .. } => {
            rewrite_operand(operand, replacements);
        }
        Rvalue::ClosureCreate { env, .. } => {
            rewrite_operand(env, replacements);
        }
    }
}

fn rewrite_callee(callee: &mut Callee, replacements: &HashMap<LocalId, LocalId>) {
    if let Callee::FnPtr(local) = callee {
        *local = rewrite_local(*local, replacements);
    }
}

fn rewrite_operand(operand: &mut Operand, replacements: &HashMap<LocalId, LocalId>) {
    match operand {
        Operand::Copy(local) | Operand::Move(local) => {
            *local = rewrite_local(*local, replacements);
        }
        Operand::Const(_) => {}
    }
}

fn rewrite_place(place: &mut Place, replacements: &HashMap<LocalId, LocalId>) {
    match place {
        Place::Local(local) | Place::Field(local, _) | Place::Deref(local) => {
            *local = rewrite_local(*local, replacements);
        }
        Place::Index(local, index) => {
            *local = rewrite_local(*local, replacements);
            rewrite_operand(index, replacements);
        }
    }
}

fn rewrite_local(local: LocalId, replacements: &HashMap<LocalId, LocalId>) -> LocalId {
    let mut current = local;
    let mut seen = HashSet::new();
    while let Some(next) = replacements.get(&current).copied() {
        if !seen.insert(current) {
            break;
        }
        current = next;
    }
    current
}

fn bump_place_write(place: &Place, counts: &mut HashMap<LocalId, u32>) {
    match place {
        Place::Local(local) | Place::Field(local, _) | Place::Index(local, _) => {
            bump_local(*local, counts)
        }
        Place::Deref(_) => {}
    }
}

fn bump_local(local: LocalId, counts: &mut HashMap<LocalId, u32>) {
    *counts.entry(local).or_insert(0) += 1;
}

use crate::{
    AirBlock, AirFunction, AirProgram, AirStmt, AirStmtKind, AirTerminator, AirType, Callee,
    LocalId, Operand, Place, Rvalue,
};
use std::collections::{HashMap, HashSet};

pub const TO_STRING: &str = "__aelys_to_string";
pub const TO_STRING_INTO: &str = "__aelys_to_string_into";
const VEC_INIT: &str = "__aelys_vec_init";
const VEC_RELEASE: &str = "__aelys_vec_release";

pub fn elide_temporaries(program: &mut AirProgram) {
    for function in &mut program.functions {
        elide_interpolation_temps(function);
        release_unbound_vec_temps(function);
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum UseKind {
    PrintArg,
    VecInitArg,
    LenOperand,
    AddrBase,
    IndexStoreBase,
    FieldBase,
    Other,
}

#[derive(Clone, Copy)]
struct UseSite {
    block: usize,
    stmt: usize,
    kind: UseKind,
}

fn is_print(callee: &Callee) -> bool {
    matches!(callee, Callee::Named(n) if n == "println" || n == "print")
}

fn is_named(callee: &Callee, want: &str) -> bool {
    matches!(callee, Callee::Named(n) if n == want)
}

fn operand_local(operand: &Operand) -> Option<LocalId> {
    match operand {
        Operand::Copy(local) | Operand::Move(local) => Some(*local),
        Operand::Const(_) => None,
    }
}

fn collect_uses(function: &AirFunction) -> HashMap<LocalId, Vec<UseSite>> {
    let mut uses: HashMap<LocalId, Vec<UseSite>> = HashMap::new();
    for (block_index, block) in function.blocks.iter().enumerate() {
        for (stmt_index, stmt) in block.stmts.iter().enumerate() {
            let mut push = |local: LocalId, kind: UseKind| {
                uses.entry(local).or_default().push(UseSite {
                    block: block_index,
                    stmt: stmt_index,
                    kind,
                });
            };
            match &stmt.kind {
                AirStmtKind::Assign { place, rvalue } => {
                    collect_place_uses(place, &mut push);
                    collect_rvalue_uses(rvalue, &mut push);
                }
                AirStmtKind::CallVoid { func, args } => {
                    collect_call_uses(func, args, &mut push);
                }
                AirStmtKind::GcAlloc { local, .. }
                | AirStmtKind::Alloc { local, .. }
                | AirStmtKind::RcAlloc { local, .. }
                | AirStmtKind::GcDrop(local)
                | AirStmtKind::Free(local) => push(*local, UseKind::Other),
                AirStmtKind::ArenaCreate(_)
                | AirStmtKind::ArenaDestroy(_)
                | AirStmtKind::MemoryFence(_) => {}
            }
        }

        let stmt_index = block.stmts.len();
        let mut push = |local: LocalId, kind: UseKind| {
            uses.entry(local).or_default().push(UseSite {
                block: block_index,
                stmt: stmt_index,
                kind,
            });
        };
        match &block.terminator {
            AirTerminator::Return(Some(op)) => {
                if let Some(local) = operand_local(op) {
                    push(local, UseKind::Other);
                }
            }
            AirTerminator::Branch { cond, .. } => {
                if let Some(local) = operand_local(cond) {
                    push(local, UseKind::Other);
                }
            }
            AirTerminator::Switch { discr, .. } => {
                if let Some(local) = operand_local(discr) {
                    push(local, UseKind::Other);
                }
            }
            AirTerminator::Invoke {
                func, args, ret, ..
            } => {
                collect_call_uses(func, args, &mut push);
                collect_place_uses(ret, &mut push);
            }
            AirTerminator::Return(None)
            | AirTerminator::Goto(_)
            | AirTerminator::Unwind
            | AirTerminator::Unreachable
            | AirTerminator::Panic { .. } => {}
        }
    }
    uses
}

fn collect_call_uses(
    func: &Callee,
    args: &[Operand],
    push: &mut impl FnMut(LocalId, UseKind),
) {
    if let Callee::FnPtr(local) = func {
        push(*local, UseKind::Other);
    }
    let kind = if is_print(func) {
        UseKind::PrintArg
    } else if is_named(func, VEC_INIT) {
        UseKind::VecInitArg
    } else {
        UseKind::Other
    };
    for arg in args {
        if let Some(local) = operand_local(arg) {
            push(local, kind);
        }
    }
}

fn collect_place_uses(place: &Place, push: &mut impl FnMut(LocalId, UseKind)) {
    match place {
        Place::Local(_) | Place::Global(_) => {}
        Place::Field(local, _) => push(*local, UseKind::Other),
        Place::Deref(local) => push(*local, UseKind::Other),
        Place::Index(local, operand) => {
            push(*local, UseKind::IndexStoreBase);
            if let Some(inner) = operand_local(operand) {
                push(inner, UseKind::Other);
            }
        }
    }
}

fn collect_rvalue_uses(rvalue: &Rvalue, push: &mut impl FnMut(LocalId, UseKind)) {
    match rvalue {
        Rvalue::Len(op) => {
            if let Some(local) = operand_local(op) {
                push(local, UseKind::LenOperand);
            }
        }
        Rvalue::FieldAccess { base, .. } => {
            if let Some(local) = operand_local(base) {
                push(local, UseKind::FieldBase);
            }
        }
        Rvalue::AddressOf(place) => match place {
            Place::Local(local) => push(*local, UseKind::AddrBase),
            other => collect_place_uses(other, push),
        },
        Rvalue::Use(op) | Rvalue::UnaryOp(_, op) | Rvalue::Deref(op) => {
            if let Some(local) = operand_local(op) {
                push(local, UseKind::Other);
            }
        }
        Rvalue::BinaryOp(_, left, right) => {
            for op in [left, right] {
                if let Some(local) = operand_local(op) {
                    push(local, UseKind::Other);
                }
            }
        }
        Rvalue::Call { func, args } => collect_call_uses(func, args, push),
        Rvalue::StructInit { fields, .. } => {
            for (_, op) in fields {
                if let Some(local) = operand_local(op) {
                    push(local, UseKind::Other);
                }
            }
        }
        Rvalue::Cast { operand, .. }
        | Rvalue::EnumTag { operand, .. }
        | Rvalue::EnumPayload { operand, .. } => {
            if let Some(local) = operand_local(operand) {
                push(local, UseKind::Other);
            }
        }
        Rvalue::Index { base, index } => {
            for op in [base, index] {
                if let Some(local) = operand_local(op) {
                    push(local, UseKind::Other);
                }
            }
        }
        Rvalue::EnumInit { payload, .. } => {
            for op in payload {
                if let Some(local) = operand_local(op) {
                    push(local, UseKind::Other);
                }
            }
        }
        Rvalue::ClosureCreate { env, .. } => {
            if let Some(local) = operand_local(env) {
                push(local, UseKind::Other);
            }
        }
        Rvalue::SliceFromParts { ptr, len } => {
            for op in [ptr, len] {
                if let Some(local) = operand_local(op) {
                    push(local, UseKind::Other);
                }
            }
        }
    }
}

fn direct_def_sites(function: &AirFunction, target: LocalId) -> Vec<(usize, usize)> {
    let mut sites = Vec::new();
    for (block_index, block) in function.blocks.iter().enumerate() {
        for (stmt_index, stmt) in block.stmts.iter().enumerate() {
            let writes = match &stmt.kind {
                AirStmtKind::Assign {
                    place: Place::Local(local),
                    ..
                } => *local == target,
                AirStmtKind::GcAlloc { local, .. }
                | AirStmtKind::Alloc { local, .. }
                | AirStmtKind::RcAlloc { local, .. } => *local == target,
                _ => false,
            };
            if writes {
                sites.push((block_index, stmt_index));
            }
        }
        if let AirTerminator::Invoke {
            ret: Place::Local(local),
            ..
        } = &block.terminator
        {
            if *local == target {
                sites.push((block_index, block.stmts.len()));
            }
        }
    }
    sites
}

fn local_type(function: &AirFunction, target: LocalId) -> Option<&AirType> {
    function
        .locals
        .iter()
        .find(|local| local.id == target)
        .map(|local| &local.ty)
        .or_else(|| {
            function
                .params
                .iter()
                .find(|param| param.id == target)
                .map(|param| &param.ty)
        })
}

fn elide_interpolation_temps(function: &mut AirFunction) {
    let uses = collect_uses(function);
    let mut rewrite: Vec<(usize, usize)> = Vec::new();

    for (block_index, block) in function.blocks.iter().enumerate() {
        for (stmt_index, stmt) in block.stmts.iter().enumerate() {
            let AirStmtKind::Assign {
                place: Place::Local(target),
                rvalue: Rvalue::Call { func, .. },
            } = &stmt.kind
            else {
                continue;
            };
            if !is_named(func, TO_STRING) {
                continue;
            }
            if !matches!(local_type(function, *target), Some(AirType::Str)) {
                continue;
            }
            if direct_def_sites(function, *target).len() != 1 {
                continue;
            }
            let sites = match uses.get(target) {
                Some(sites) => sites,
                None => continue,
            };
            let [only] = sites.as_slice() else { continue };
            if only.kind != UseKind::PrintArg || only.block != block_index {
                continue;
            }
            rewrite.push((block_index, stmt_index));
        }
    }

    for (block_index, stmt_index) in rewrite {
        if let AirStmtKind::Assign {
            rvalue: Rvalue::Call { func, .. },
            ..
        } = &mut function.blocks[block_index].stmts[stmt_index].kind
        {
            *func = Callee::Named(TO_STRING_INTO.to_string());
        }
    }
}

/// a literal's temp that is never handed on still owns the +1 vec_init took, and nobody frees it
fn release_unbound_vec_temps(function: &mut AirFunction) {
    let uses = collect_uses(function);
    let mut inserts: Vec<(usize, usize, LocalId)> = Vec::new();
    let mut released: HashSet<LocalId> = HashSet::new();

    for (block_index, block) in function.blocks.iter().enumerate() {
        for stmt in &block.stmts {
            let AirStmtKind::CallVoid { func, args } = &stmt.kind else {
                continue;
            };
            if !is_named(func, VEC_INIT) {
                continue;
            }
            let Some(addr_local) = args.first().and_then(operand_local) else {
                continue;
            };
            let Some(target) = address_taken_local(function, addr_local) else {
                continue;
            };
            if !matches!(local_type(function, target), Some(AirType::Vec(_))) {
                continue;
            }
            if direct_def_sites(function, addr_local).len() != 1 {
                continue;
            }
            if !direct_def_sites(function, target).is_empty() {
                continue;
            }

            let Some(target_uses) = uses.get(&target) else {
                continue;
            };
            // any read not in this set could hand the share on, and releasing then double-frees
            let consumed_in_place = target_uses.iter().all(|site| {
                site.block == block_index
                    && matches!(
                        site.kind,
                        UseKind::AddrBase | UseKind::IndexStoreBase | UseKind::FieldBase
                    )
            });
            if !consumed_in_place {
                continue;
            }

            let addr_uses = uses.get(&addr_local).map(Vec::as_slice).unwrap_or(&[]);
            let addr_is_inert = addr_uses.iter().all(|site| {
                site.block == block_index
                    && matches!(site.kind, UseKind::VecInitArg | UseKind::LenOperand)
            });
            if !addr_is_inert {
                continue;
            }

            let last_use = target_uses
                .iter()
                .chain(addr_uses.iter())
                .map(|site| site.stmt)
                .max()
                .unwrap_or(0);
            if last_use >= block.stmts.len() {
                continue;
            }
            if !released.insert(target) {
                continue;
            }
            inserts.push((block_index, last_use + 1, addr_local));
        }
    }

    apply_release_inserts(function, inserts);
}

fn address_taken_local(function: &AirFunction, addr_local: LocalId) -> Option<LocalId> {
    for block in &function.blocks {
        for stmt in &block.stmts {
            if let AirStmtKind::Assign {
                place: Place::Local(local),
                rvalue: Rvalue::AddressOf(Place::Local(base)),
            } = &stmt.kind
            {
                if *local == addr_local {
                    return Some(*base);
                }
            }
        }
    }
    None
}

fn apply_release_inserts(function: &mut AirFunction, mut inserts: Vec<(usize, usize, LocalId)>) {
    if inserts.is_empty() {
        return;
    }
    inserts.sort_by(|a, b| b.1.cmp(&a.1));
    for (block_index, at, addr_local) in inserts {
        let block: &mut AirBlock = &mut function.blocks[block_index];
        let span = block.stmts.get(at.saturating_sub(1)).and_then(|s| s.span);
        block.stmts.insert(
            at,
            AirStmt {
                kind: AirStmtKind::CallVoid {
                    func: Callee::Named(VEC_RELEASE.to_string()),
                    args: vec![Operand::Copy(addr_local)],
                },
                span,
            },
        );
    }
}

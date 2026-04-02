use crate::{
    AirFunction, AirProgram, AirStmtKind, AirTerminator, Callee, LocalId, Operand, Place, Rvalue,
};
use std::collections::HashSet;

pub fn eliminate_dead_locals(program: &mut AirProgram) {
    for function in &mut program.functions {
        eliminate_function_dead_locals(function);
    }
}

fn eliminate_function_dead_locals(function: &mut AirFunction) {
    let mut referenced = HashSet::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            collect_stmt_locals(&stmt.kind, &mut referenced);
        }
        collect_terminator_locals(&block.terminator, &mut referenced);
    }

    // remove dead locals from the locals list
    function
        .locals
        .retain(|local| referenced.contains(&local.id));

    // Transform or remove statements that assign to dead locals, dead store elimination
    for block in &mut function.blocks {
        for stmt in &mut block.stmts {
            if let AirStmtKind::Assign {
                place: Place::Local(local),
                rvalue,
            } = &stmt.kind
            {
                if !referenced.contains(local) {
                    // Dead store - transform Call into CallVoid (preserves side effects)
                    if let Rvalue::Call { func, args } = rvalue {
                        stmt.kind = AirStmtKind::CallVoid {
                            func: func.clone(),
                            args: args.clone(),
                        };
                    }
                    // other rvalues without side effects can be left as-is and will be removed in a second pass
                }
            }
        }

        // remove remaining dead stores (non-Call assigns to dead locals)
        block.stmts.retain(|stmt| {
            if let AirStmtKind::Assign {
                place: Place::Local(local),
                ..
            } = &stmt.kind
            {
                referenced.contains(local)
            } else {
                true
            }
        });
    }
}

fn collect_stmt_locals(stmt: &AirStmtKind, out: &mut HashSet<LocalId>) {
    match stmt {
        AirStmtKind::Assign { place, rvalue } => {
            collect_place_locals(place, out);
            collect_rvalue_locals(rvalue, out);
        }
        AirStmtKind::GcAlloc { local, .. } | AirStmtKind::Alloc { local, .. } => {
            out.insert(*local);
        }
        AirStmtKind::GcDrop(local) | AirStmtKind::Free(local) => {
            out.insert(*local);
        }
        AirStmtKind::CallVoid { func, args } => {
            collect_callee_locals(func, out);
            for arg in args {
                collect_operand_locals(arg, out);
            }
        }
        AirStmtKind::ArenaCreate(_)
        | AirStmtKind::ArenaDestroy(_)
        | AirStmtKind::MemoryFence(_) => {}
    }
}

fn collect_terminator_locals(term: &AirTerminator, out: &mut HashSet<LocalId>) {
    match term {
        AirTerminator::Return(Some(op)) => collect_operand_locals(op, out),
        AirTerminator::Branch { cond, .. } => collect_operand_locals(cond, out),
        AirTerminator::Switch { discr, .. } => collect_operand_locals(discr, out),
        AirTerminator::Invoke {
            func, args, ret, ..
        } => {
            collect_callee_locals(func, out);
            for arg in args {
                collect_operand_locals(arg, out);
            }
            collect_place_locals(ret, out);
        }
        AirTerminator::Return(None)
        | AirTerminator::Goto(_)
        | AirTerminator::Unwind
        | AirTerminator::Unreachable
        | AirTerminator::Panic { .. } => {}
    }
}

fn collect_rvalue_locals(rvalue: &Rvalue, out: &mut HashSet<LocalId>) {
    match rvalue {
        Rvalue::Use(op) | Rvalue::UnaryOp(_, op) | Rvalue::Deref(op) => {
            collect_operand_locals(op, out);
        }
        Rvalue::BinaryOp(_, left, right) => {
            collect_operand_locals(left, out);
            collect_operand_locals(right, out);
        }
        Rvalue::Call { func, args } => {
            collect_callee_locals(func, out);
            for arg in args {
                collect_operand_locals(arg, out);
            }
        }
        Rvalue::StructInit { fields, .. } => {
            for (_, operand) in fields {
                collect_operand_locals(operand, out);
            }
        }
        Rvalue::FieldAccess { base, .. } | Rvalue::Cast { operand: base, .. } => {
            collect_operand_locals(base, out);
        }
        Rvalue::Index { base, index } => {
            collect_operand_locals(base, out);
            collect_operand_locals(index, out);
        }
        Rvalue::AddressOf(local) => {
            out.insert(*local);
        }
        Rvalue::EnumInit { payload, .. } => {
            for operand in payload {
                collect_operand_locals(operand, out);
            }
        }
        Rvalue::EnumTag { operand, .. } => {
            collect_operand_locals(operand, out);
        }
        Rvalue::EnumPayload { operand, .. } => {
            collect_operand_locals(operand, out);
        }
        Rvalue::ClosureCreate { env, .. } => {
            collect_operand_locals(env, out);
        }
    }
}

fn collect_place_locals(place: &Place, out: &mut HashSet<LocalId>) {
    match place {
        Place::Local(_) => {
            // this is just a write destination, don't mark as referenced, only Field/Index/Deref need the base local to exist because they read it
        }
        Place::Field(local, _) | Place::Deref(local) => {
            // must read the base to access field/deref
            out.insert(*local);
        }
        Place::Index(local, operand) => {
            // same thing
            out.insert(*local);
            collect_operand_locals(operand, out); // index expression uses this operand
        }
    }
}

fn collect_operand_locals(operand: &Operand, out: &mut HashSet<LocalId>) {
    match operand {
        Operand::Copy(local) | Operand::Move(local) => {
            out.insert(*local);
        }
        Operand::Const(_) => {}
    }
}

fn collect_callee_locals(callee: &Callee, out: &mut HashSet<LocalId>) {
    if let Callee::FnPtr(local) = callee {
        out.insert(*local);
    }
}

use crate::{
    AirConst, AirFunction, AirProgram, AirStmtKind, AirTerminator, Callee, Operand, Place, Rvalue,
};
use std::collections::HashMap;

const GLOBAL_GET: &str = "__aelys_global_get_";
const GLOBAL_SET: &str = "__aelys_global_set_";

pub fn drop_unreached(program: &mut AirProgram, candidate: impl Fn(&AirFunction) -> bool) {
    let by_name: HashMap<&str, usize> = program
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.name.as_str(), i))
        .collect();
    let by_id: HashMap<u32, usize> = program
        .functions
        .iter()
        .enumerate()
        .map(|(i, f)| (f.id.0, i))
        .collect();
    let global_by_name: HashMap<&str, usize> = program
        .globals
        .iter()
        .enumerate()
        .map(|(i, g)| (g.name.as_str(), i))
        .collect();
    let mut stack: Vec<usize> = (0..program.functions.len())
        .filter(|&i| {
            let function = &program.functions[i];
            function.is_extern || !candidate(function)
        })
        .collect();
    let mut names: Vec<&str> = program
        .dyn_tables
        .vtables
        .iter()
        .flat_map(|vt| vt.fns.iter().map(String::as_str))
        .collect();
    let mut live = vec![false; program.functions.len()];
    let mut live_globals = vec![false; program.globals.len()];
    loop {
        while let Some(name) = names.pop() {
            if let Some(&i) = by_name.get(name) {
                stack.push(i);
            }
            let global = name
                .strip_prefix(GLOBAL_GET)
                .or_else(|| name.strip_prefix(GLOBAL_SET))
                .unwrap_or(name);
            if let Some(&g) = global_by_name.get(global)
                && !std::mem::replace(&mut live_globals[g], true)
                && let Some(init) = &program.globals[g].init
            {
                const_names(init, &mut names);
            }
        }
        let Some(i) = stack.pop() else { break };
        if std::mem::replace(&mut live[i], true) {
            continue;
        }
        let mut ids = Vec::new();
        function_refs(&program.functions[i], &mut names, &mut ids);
        stack.extend(ids.into_iter().filter_map(|id| by_id.get(&id).copied()));
    }
    let mut kept = live_globals.iter();
    program.globals.retain(|_| *kept.next().unwrap());
    if live.iter().all(|&l| l) {
        return;
    }
    let mut kept = live.iter();
    program.functions.retain(|_| *kept.next().unwrap());
    let ids: std::collections::HashSet<_> = program.functions.iter().map(|f| f.id).collect();
    program.mono_instances.retain(|m| ids.contains(&m.result));
}

fn function_refs<'a>(function: &'a AirFunction, names: &mut Vec<&'a str>, ids: &mut Vec<u32>) {
    for block in &function.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                AirStmtKind::Assign { place, rvalue } => {
                    place_names(place, names);
                    rvalue_refs(rvalue, names, ids);
                }
                AirStmtKind::CallVoid { func, args } => {
                    callee_refs(func, names, ids);
                    args.iter().for_each(|a| operand_names(a, names));
                }
                AirStmtKind::GcAlloc { .. }
                | AirStmtKind::GcDrop(_)
                | AirStmtKind::ArenaCreate(_)
                | AirStmtKind::ArenaDestroy(_)
                | AirStmtKind::Alloc { .. }
                | AirStmtKind::RcAlloc { .. }
                | AirStmtKind::Free(_)
                | AirStmtKind::MemoryFence(_) => {}
            }
        }
        match &block.terminator {
            AirTerminator::Return(Some(op)) => operand_names(op, names),
            AirTerminator::Branch { cond, .. } => operand_names(cond, names),
            AirTerminator::Switch { discr, targets, .. } => {
                operand_names(discr, names);
                targets.iter().for_each(|(c, _)| const_names(c, names));
            }
            AirTerminator::Invoke {
                func, args, ret, ..
            } => {
                callee_refs(func, names, ids);
                args.iter().for_each(|a| operand_names(a, names));
                place_names(ret, names);
            }
            AirTerminator::Return(None)
            | AirTerminator::Goto(_)
            | AirTerminator::Unwind
            | AirTerminator::Unreachable
            | AirTerminator::Panic { .. } => {}
        }
    }
}

fn callee_refs<'a>(callee: &'a Callee, names: &mut Vec<&'a str>, ids: &mut Vec<u32>) {
    match callee {
        Callee::Direct(id) => ids.push(id.0),
        Callee::Named(name) | Callee::Extern(name, _) => names.push(name),
        Callee::FnPtr(_) => {}
    }
}

fn rvalue_refs<'a>(rvalue: &'a Rvalue, names: &mut Vec<&'a str>, ids: &mut Vec<u32>) {
    match rvalue {
        Rvalue::Use(op)
        | Rvalue::UnaryOp(_, op)
        | Rvalue::FieldAccess { base: op, .. }
        | Rvalue::Deref(op)
        | Rvalue::Cast { operand: op, .. }
        | Rvalue::EnumTag { operand: op, .. }
        | Rvalue::EnumPayload { operand: op, .. }
        | Rvalue::Len(op) => operand_names(op, names),
        Rvalue::BinaryOp(_, a, b)
        | Rvalue::Index { base: a, index: b }
        | Rvalue::SliceFromParts { ptr: a, len: b } => {
            operand_names(a, names);
            operand_names(b, names);
        }
        Rvalue::Call { func, args } => {
            callee_refs(func, names, ids);
            args.iter().for_each(|a| operand_names(a, names));
        }
        Rvalue::StructInit { fields, .. } => {
            fields.iter().for_each(|(_, a)| operand_names(a, names));
        }
        Rvalue::EnumInit { payload, .. } => payload.iter().for_each(|a| operand_names(a, names)),
        Rvalue::AddressOf(place) => place_names(place, names),
        Rvalue::ClosureCreate { fn_name, env } => {
            names.push(fn_name);
            operand_names(env, names);
        }
    }
}

fn place_names<'a>(place: &'a Place, names: &mut Vec<&'a str>) {
    match place {
        Place::Global(name) => names.push(name),
        Place::Index(_, op) => operand_names(op, names),
        Place::Local(_) | Place::Field(..) | Place::Deref(_) => {}
    }
}

fn operand_names<'a>(operand: &'a Operand, names: &mut Vec<&'a str>) {
    if let Operand::Const(c) = operand {
        const_names(c, names);
    }
}

fn const_names<'a>(c: &'a AirConst, names: &mut Vec<&'a str>) {
    match c {
        AirConst::FnRef(name) => names.push(name),
        AirConst::Enum { payload, .. } | AirConst::Array(payload) => {
            payload.iter().for_each(|c| const_names(c, names));
        }
        AirConst::Struct { fields, .. } => fields.iter().for_each(|(_, c)| const_names(c, names)),
        AirConst::IntLiteral(_)
        | AirConst::Int(..)
        | AirConst::Float(..)
        | AirConst::Bool(_)
        | AirConst::Char(_)
        | AirConst::Str(_)
        | AirConst::Null
        | AirConst::ZeroInit(_)
        | AirConst::Undef(_) => {}
    }
}

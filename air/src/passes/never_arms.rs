use crate::analysis::cfg::successors;
use crate::{AirFunction, AirProgram, AirStmtKind, AirTerminator, AirType, Place, Rvalue};
use std::collections::HashSet;

pub fn cut_never_arms(program: &mut AirProgram) {
    let mut cut = false;
    for function in &mut program.functions {
        cut |= cut_function_never_arms(function);
    }
    if cut {
        drop_orphan_instances(program);
    }
}

// an instance only a cut arm reached would still be emitted with its void locals
fn drop_orphan_instances(program: &mut AirProgram) {
    let instances: HashSet<_> = program.mono_instances.iter().map(|i| i.result).collect();
    let texts: Vec<String> = program
        .functions
        .iter()
        .map(|f| crate::print::print_function(f, program))
        .collect();
    let mut roots: Vec<String> = program
        .globals
        .iter()
        .filter_map(|g| g.init.as_ref().map(crate::print::fmt_const))
        .collect();
    let mut live: HashSet<usize> = HashSet::new();
    for (i, f) in program.functions.iter().enumerate() {
        if !instances.contains(&f.id) {
            live.insert(i);
            roots.push(texts[i].clone());
        }
    }
    let mut frontier = roots;
    while !frontier.is_empty() {
        let mut next = Vec::new();
        for (i, f) in program.functions.iter().enumerate() {
            if !live.contains(&i) && frontier.iter().any(|t| t.contains(f.name.as_str())) {
                live.insert(i);
                next.push(texts[i].clone());
            }
        }
        frontier = next;
    }
    let dead: HashSet<_> = program
        .functions
        .iter()
        .enumerate()
        .filter(|(i, _)| !live.contains(i))
        .map(|(_, f)| f.id)
        .collect();
    if dead.is_empty() {
        return;
    }
    program.functions.retain(|f| !dead.contains(&f.id));
    program.mono_instances.retain(|m| !dead.contains(&m.result));
}

fn cut_function_never_arms(function: &mut AirFunction) -> bool {
    let never: HashSet<_> = function
        .locals
        .iter()
        .filter(|l| l.ty == AirType::Void)
        .map(|l| l.id)
        .collect();
    if never.is_empty() {
        return false;
    }
    let mut cut = false;
    for block in &mut function.blocks {
        let first = block.stmts.iter().position(|stmt| {
            matches!(
                &stmt.kind,
                AirStmtKind::Assign {
                    place: Place::Local(local),
                    rvalue: Rvalue::EnumPayload { .. },
                } if never.contains(local)
            )
        });
        if let Some(index) = first {
            block.stmts.truncate(index);
            block.terminator = AirTerminator::Unreachable;
            cut = true;
        }
    }
    if !cut || function.blocks.is_empty() {
        return cut;
    }
    let mut reached = HashSet::new();
    let mut work = vec![function.blocks[0].id];
    while let Some(id) = work.pop() {
        if !reached.insert(id) {
            continue;
        }
        if let Some(block) = function.blocks.iter().find(|b| b.id == id) {
            work.extend(successors(&block.terminator));
        }
    }
    for block in &mut function.blocks {
        if !reached.contains(&block.id) {
            block.stmts.clear();
            block.terminator = AirTerminator::Unreachable;
        }
    }
    true
}

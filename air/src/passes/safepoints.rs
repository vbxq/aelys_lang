use crate::{
    AirBlock, AirFunction, AirProgram, AirStmt, AirStmtKind, AirTerminator, BlockId, Callee,
    LocalId, Operand, Place, Rvalue,
};
use std::collections::{HashMap, HashSet};

pub const CYCLE_POLL: &str = "__aelys_cycle_poll";
const COLLECT: &str = "__aelys_collect";

fn poll_stmt() -> AirStmt {
    AirStmt {
        kind: AirStmtKind::CallVoid {
            func: Callee::Named(CYCLE_POLL.to_string()),
            args: Vec::new(),
        },
        span: None,
    }
}

pub fn insert_cycle_polls(program: &mut AirProgram) {
    for function in &mut program.functions {
        if function.is_extern || function.blocks.is_empty() || function.attributes.no_gc {
            continue;
        }
        poll_function(function);
    }
}

// must agree with the entry the codegen picks, or the entry poll lands in a loop body
fn entry_block(function: &AirFunction) -> BlockId {
    let mut targeted = HashSet::new();
    for block in &function.blocks {
        match &block.terminator {
            AirTerminator::Goto(t) => {
                targeted.insert(*t);
            }
            AirTerminator::Branch {
                then_block,
                else_block,
                ..
            } => {
                targeted.insert(*then_block);
                targeted.insert(*else_block);
            }
            AirTerminator::Switch {
                targets, default, ..
            } => {
                targeted.extend(targets.iter().map(|(_, b)| *b));
                targeted.insert(*default);
            }
            _ => {}
        }
    }
    function
        .blocks
        .iter()
        .find(|b| !targeted.contains(&b.id))
        .unwrap_or(&function.blocks[0])
        .id
}

fn retreating_edges(function: &AirFunction, entry: BlockId) -> Vec<(BlockId, BlockId)> {
    let index = crate::analysis::cfg::block_index_map(function);
    let mut state: HashMap<BlockId, u8> = HashMap::new();
    let mut edges = Vec::new();
    let mut stack: Vec<(BlockId, Vec<BlockId>, usize)> = Vec::new();
    let succ = |b: BlockId| -> Vec<BlockId> {
        index
            .get(&b)
            .map(|&i| crate::analysis::cfg::successors(&function.blocks[i].terminator))
            .unwrap_or_default()
    };
    state.insert(entry, 1);
    stack.push((entry, succ(entry), 0));
    while let Some((node, succs, next)) = stack.last_mut() {
        if *next == succs.len() {
            state.insert(*node, 2);
            stack.pop();
            continue;
        }
        let s = succs[*next];
        *next += 1;
        let from = *node;
        match state.get(&s).copied().unwrap_or(0) {
            0 if index.contains_key(&s) => {
                state.insert(s, 1);
                let ss = succ(s);
                stack.push((s, ss, 0));
            }
            1 => {
                if !edges.contains(&(from, s)) {
                    edges.push((from, s));
                }
            }
            _ => {}
        }
    }
    edges
}

fn retarget(term: &mut AirTerminator, from: BlockId, to: BlockId) {
    let swap = |b: &mut BlockId| {
        if *b == from {
            *b = to;
        }
    };
    match term {
        AirTerminator::Goto(t) => swap(t),
        AirTerminator::Branch {
            then_block,
            else_block,
            ..
        } => {
            swap(then_block);
            swap(else_block);
        }
        AirTerminator::Switch {
            targets, default, ..
        } => {
            for (_, b) in targets.iter_mut() {
                swap(b);
            }
            swap(default);
        }
        AirTerminator::Invoke { normal, unwind, .. } => {
            swap(normal);
            swap(unwind);
        }
        AirTerminator::Return(_)
        | AirTerminator::Unwind
        | AirTerminator::Unreachable
        | AirTerminator::Panic { .. } => {}
    }
}

fn poll_function(function: &mut AirFunction) {
    let entry = entry_block(function);
    let edges = retreating_edges(function, entry);
    let mut next_id = function.blocks.iter().map(|b| b.id.0).max().unwrap_or(0) + 1;
    for (from, to) in edges {
        let Some(i) = function.blocks.iter().position(|b| b.id == from) else {
            continue;
        };
        if matches!(function.blocks[i].terminator, AirTerminator::Goto(t) if t == to) {
            function.blocks[i].stmts.push(poll_stmt());
            continue;
        }
        let split = BlockId(next_id);
        next_id += 1;
        retarget(&mut function.blocks[i].terminator, to, split);
        function.blocks.push(AirBlock {
            id: split,
            stmts: vec![poll_stmt()],
            terminator: AirTerminator::Goto(to),
        });
    }
    if let Some(b) = function.blocks.iter_mut().find(|b| b.id == entry) {
        b.stmts.insert(0, poll_stmt());
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
enum Sel {
    Field(String),
    Index(Option<String>),
    Deref,
}

#[derive(Clone, Debug)]
struct Slot {
    base: LocalId,
    sel: Sel,
}

impl Slot {
    fn same(&self, other: &Slot) -> bool {
        if self.base != other.base {
            return false;
        }
        match (&self.sel, &other.sel) {
            (Sel::Index(a), Sel::Index(b)) => a.is_none() || b.is_none() || a == b,
            (a, b) => a == b,
        }
    }
}

fn operand_local(op: &Operand) -> Option<LocalId> {
    match op {
        Operand::Copy(l) | Operand::Move(l) => Some(*l),
        Operand::Const(_) => None,
    }
}

fn index_key(op: &Operand) -> Option<String> {
    match op {
        Operand::Copy(l) | Operand::Move(l) => Some(format!("%{}", l.0)),
        Operand::Const(crate::AirConst::IntLiteral(v) | crate::AirConst::Int(v, _)) => {
            Some(format!("#{v}"))
        }
        Operand::Const(_) => None,
    }
}

fn place_slot(place: &Place) -> Option<Slot> {
    match place {
        Place::Field(b, f) => Some(Slot {
            base: *b,
            sel: Sel::Field(f.clone()),
        }),
        Place::Index(b, i) => Some(Slot {
            base: *b,
            sel: Sel::Index(index_key(i)),
        }),
        Place::Deref(b) => Some(Slot {
            base: *b,
            sel: Sel::Deref,
        }),
        Place::Local(_) | Place::Global(_) => None,
    }
}

fn load_slot(rvalue: &Rvalue) -> Option<Slot> {
    match rvalue {
        Rvalue::FieldAccess { base, field } => Some(Slot {
            base: operand_local(base)?,
            sel: Sel::Field(field.clone()),
        }),
        Rvalue::Index { base, index } => Some(Slot {
            base: operand_local(base)?,
            sel: Sel::Index(index_key(index)),
        }),
        Rvalue::Deref(base) => Some(Slot {
            base: operand_local(base)?,
            sel: Sel::Deref,
        }),
        _ => None,
    }
}

fn is_release_call(name: &str) -> bool {
    crate::counts::is_release(name)
        || name == "__aelys_rc_release"
        || name == "__aelys_rc_release_slot"
        || (name.starts_with("__glue_") && name.contains("drop"))
}

fn runs_user_code(callee: &Callee) -> bool {
    match callee {
        Callee::Direct(_) | Callee::FnPtr(_) | Callee::Extern(..) => true,
        Callee::Named(n) => {
            n == CYCLE_POLL
                || n == COLLECT
                || !(n.starts_with("__aelys_") || n.starts_with("__glue_"))
        }
    }
}

fn stmt_call(kind: &AirStmtKind) -> Option<(&Callee, &[Operand])> {
    match kind {
        AirStmtKind::CallVoid { func, args } => Some((func, args)),
        AirStmtKind::Assign {
            rvalue: Rvalue::Call { func, args },
            ..
        } => Some((func, args)),
        _ => None,
    }
}

fn callee_label(callee: &Callee) -> String {
    match callee {
        Callee::Direct(id) => format!("function #{}", id.0),
        Callee::Named(n) | Callee::Extern(n, _) => n.clone(),
        Callee::FnPtr(l) => format!("closure %{}", l.0),
    }
}

pub struct ReleaseWindows {
    pub checked: usize,
    pub violations: Vec<String>,
}

fn block_windows(function: &AirFunction, block: &AirBlock, out: &mut ReleaseWindows) {
    let mut loaded: HashMap<LocalId, Slot> = HashMap::new();
    let mut open: Vec<(Slot, usize)> = Vec::new();
    for (i, stmt) in block.stmts.iter().enumerate() {
        if let AirStmtKind::Assign { place, rvalue } = &stmt.kind {
            if let Place::Local(dst) = place {
                loaded.remove(dst);
                if let Some(slot) = load_slot(rvalue) {
                    loaded.insert(*dst, slot);
                } else if let Rvalue::AddressOf(target) = rvalue {
                    let slot = match target {
                        Place::Local(l) => loaded.get(l).cloned(),
                        other => place_slot(other),
                    };
                    if let Some(slot) = slot {
                        loaded.insert(*dst, slot);
                    }
                }
            } else if let Some(stored) = place_slot(place) {
                let mut k = 0;
                while k < open.len() {
                    if !open[k].0.same(&stored) {
                        k += 1;
                        continue;
                    }
                    let (_, start) = open.remove(k);
                    out.checked += 1;
                    for between in &block.stmts[start + 1..i] {
                        if let Some((callee, _)) = stmt_call(&between.kind)
                            && runs_user_code(callee)
                        {
                            out.violations.push(format!(
                                "fn {}: block{} calls {} between the release of an old value \
                                 (stmt {}) and the store replacing it (stmt {})",
                                function.name,
                                block.id.0,
                                callee_label(callee),
                                start,
                                i
                            ));
                        }
                    }
                }
            }
        }
        if let Some((Callee::Named(name), args)) = stmt_call(&stmt.kind)
            && is_release_call(name)
            && let Some(arg) = args.first().and_then(operand_local)
            && let Some(slot) = loaded.get(&arg)
        {
            open.push((slot.clone(), i));
        }
    }
}

// a safepoint inside this window would trace a field whose old target is already freed
pub fn release_store_windows(program: &AirProgram) -> ReleaseWindows {
    let mut out = ReleaseWindows {
        checked: 0,
        violations: Vec::new(),
    };
    for function in &program.functions {
        if function.is_extern {
            continue;
        }
        for block in &function.blocks {
            block_windows(function, block, &mut out);
        }
    }
    out
}

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;

use aelys_syntax::Span;

use super::origins::{Summaries, SummaryEntry};
use super::*;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LoanId(pub(super) u32);

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoanKind {
    Shared,
    Mut,
}

// pub(super) fields are the seeds pass 1 (origins.rs) reads: id, borrowed place, holder, reborrow
pub(super) struct Loan {
    pub(super) id: LoanId,
    // the borrowed place, e.g. {v,[index]} for &v[0]
    pub(super) place: BirPlace,
    kind: LoanKind,
    block: BirBlockId,
    index: usize,
    // borrow-expression span, for the [borrow] message
    span: Span,
    pub(super) holder: BirLocalId,
    // some(r) iff this is &mut *r, so the reborrow inherits r's loans (suspend rule)
    pub(super) reborrow_base: Option<BirLocalId>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Access {
    Read,
    Write,
    Move,
    BorrowShared,
    BorrowMut,
}

pub fn check(bir: &BirProgram, summaries: &Summaries) -> Vec<BirDiagnostic> {
    let mut errors = Vec::new();
    for body in &bir.bodies {
        check_body(body, summaries, &mut errors);
    }
    errors
}

fn local_is_ref(body: &BirBody, l: BirLocalId) -> bool {
    body.locals
        .get(l.0 as usize)
        .map(|loc| is_ref_ty(&loc.ty))
        .unwrap_or(false)
}

// e0714 asks whether the operand is a reference, and a projection off a reference base need not be one
fn projected_is_ref(body: &BirBody, place: &BirPlace) -> bool {
    let Some(local) = body.locals.get(place.local.0 as usize) else {
        return false;
    };
    let mut ty = &local.ty;
    for step in &place.proj {
        ty = match (step, ty) {
            (BirProjection::Deref, InferType::Ref { referent, .. }) => referent,
            (
                BirProjection::Index,
                InferType::Slice { elem, .. } | InferType::Array(elem, _) | InferType::Vec(elem),
            ) => elem,
            _ => return is_ref_ty(&local.ty),
        };
    }
    is_ref_ty(ty)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RootClass {
    Managed,
    Unmanaged,
    Unknown,
}

fn classify_loan_root(body: &BirBody, place: &BirPlace) -> RootClass {
    if place
        .proj
        .iter()
        .any(|projection| matches!(projection, BirProjection::Field(_)))
    {
        return RootClass::Unknown;
    }

    let local = &body.locals[place.local.0 as usize];
    let mut peeled = false;
    let mut ty = &local.ty;
    while let InferType::Ref { referent, .. } = ty {
        ty = referent;
        peeled = true;
    }

    match ty {
        InferType::Vec(_) | InferType::Rc(_) => RootClass::Managed,
        InferType::Slice { .. } => RootClass::Unknown,
        InferType::Struct(_)
        | InferType::Enum(_, _)
        | InferType::Tuple(_)
        | InferType::Array(_, _) => {
            if peeled {
                RootClass::Unknown
            } else if local.category.is_managed() {
                RootClass::Managed
            } else {
                RootClass::Unmanaged
            }
        }
        InferType::Var(_) | InferType::Dynamic => RootClass::Unknown,
        InferType::I8
        | InferType::I16
        | InferType::I32
        | InferType::I64
        | InferType::U8
        | InferType::U16
        | InferType::U32
        | InferType::U64
        | InferType::F32
        | InferType::F64
        | InferType::Bool
        | InferType::String
        | InferType::Null
        | InferType::Never
        | InferType::Function { .. }
        | InferType::Range => RootClass::Unmanaged,
        InferType::Ref { .. } => RootClass::Unknown,
    }
}

fn format_loan_projection(projections: &[BirProjection]) -> String {
    let mut out = String::from("[");
    for (index, projection) in projections.iter().enumerate() {
        if index != 0 {
            out.push(',');
        }
        match projection {
            BirProjection::Field(name) => {
                out.push_str("Field(");
                out.push_str(name);
                out.push(')');
            }
            BirProjection::Index => out.push_str("Index"),
            BirProjection::Deref => out.push_str("Deref"),
        }
    }
    out.push(']');
    out
}

fn dump_loan_roots(body: &BirBody, loans: &[Loan]) {
    let Some(target) = std::env::var_os("AELYS_DUMP_LOAN_ROOTS") else {
        return;
    };
    if target.is_empty() {
        return;
    }
    let path = if target == "1" { None } else { Some(target) };
    let mut text = String::new();
    for loan in loans {
        let Some(local) = body.locals.get(loan.place.local.0 as usize) else {
            continue;
        };
        text.push_str("LOANROOT fn=");
        text.push_str(&body.name);
        text.push_str(" loan=");
        text.push_str(&loan.id.0.to_string());
        text.push_str(" root=%");
        text.push_str(&loan.place.local.0.to_string());
        text.push_str(" proj=");
        text.push_str(&format_loan_projection(&loan.place.proj));
        text.push_str(" ty=");
        text.push_str(&format!("{:?}", local.ty));
        text.push_str(" class=");
        text.push_str(&format!("{:?}", classify_loan_root(body, &loan.place)));
        text.push('\n');
    }
    if let Some(path) = path {
        match OpenOptions::new().create(true).append(true).open(path) {
            Ok(mut file) => {
                let _ = file.write_all(text.as_bytes());
            }
            Err(error) => eprintln!("loan-root dump unavailable: {error}"),
        }
    } else {
        eprint!("{text}");
    }
}

fn check_body(body: &BirBody, summaries: &Summaries, errors: &mut Vec<BirDiagnostic>) {
    let loans = gen_loans(body, errors);
    dump_loan_roots(body, &loans);
    // no borrows form zero loans, so the whole pass is a no-op (managed byte-identity rests here);
    if loans.is_empty() {
        return;
    }
    let holds = compute_holds(body, &loans, summaries);
    let live = compute_liveness(body);
    check_conflicts(body, &loans, &holds, &live, errors);
    check_scope_deaths(body, &loans, &holds, &live, errors);
}

pub(super) fn gen_loans(body: &BirBody, errors: &mut Vec<BirDiagnostic>) -> Vec<Loan> {
    let mut loans = Vec::new();
    for block in &body.blocks {
        for (i, stmt) in block.stmts.iter().enumerate() {
            let BirStmtKind::Assign { dest, rvalue } = &stmt.kind else {
                continue;
            };
            match rvalue {
                BirRvalue::Ref { place, mutable } | BirRvalue::Reborrow { place, mutable } => {
                    loans.push(Loan {
                        id: LoanId(loans.len() as u32),
                        place: place.clone(),
                        kind: if *mutable {
                            LoanKind::Mut
                        } else {
                            LoanKind::Shared
                        },
                        block: block.id,
                        index: i,
                        span: stmt.span,
                        holder: dest.local,
                        reborrow_base: reborrow_base_of(body, place),
                    });
                }
                BirRvalue::Aggregate(ops) => {
                    for op in ops {
                        if let BirOperand::Copy(p) | BirOperand::Move(p) = op {
                            if projected_is_ref(body, p) {
                                let decl = body.locals[p.local.0 as usize].decl_span;
                                errors.push(
                                    BirDiagnostic::new(
                                        "E0714",
                                        "[borrow]",
                                        stmt.span,
                                        "[borrow] references stored into aggregate containers are \
                                         not supported in Run 1"
                                            .to_string(),
                                    )
                                    .with_secondary(decl, "this reference".to_string()),
                                );
                                break;
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
    loans
}

// a borrow is a reborrow of r iff it goes through a leading deref of a reference-typed base
fn reborrow_base_of(body: &BirBody, place: &BirPlace) -> Option<BirLocalId> {
    if matches!(place.proj.first(), Some(BirProjection::Deref)) && local_is_ref(body, place.local) {
        Some(place.local)
    } else {
        None
    }
}

// ---- provenance: forward monotone points-to over reference copies + reborrow inheritance

fn compute_holds(body: &BirBody, loans: &[Loan], summaries: &Summaries) -> Vec<HashSet<u32>> {
    let n = body.locals.len();
    let mut holds: Vec<HashSet<u32>> = vec![HashSet::new(); n];

    for l in loans {
        if (l.holder.0 as usize) < n {
            holds[l.holder.0 as usize].insert(l.id.0);
        }
    }

    // dst <- src flow edges: copy/move of a whole-local reference, and reborrow inheritance
    let mut edges: Vec<(usize, usize)> = Vec::new();
    for block in &body.blocks {
        for stmt in &block.stmts {
            if let BirStmtKind::Assign { dest, rvalue } = &stmt.kind {
                if dest.proj.is_empty() {
                    if let BirRvalue::Use(BirOperand::Copy(s) | BirOperand::Move(s)) = rvalue {
                        if s.proj.is_empty() {
                            edges.push((dest.local.0 as usize, s.local.0 as usize));
                        }
                    }
                }
            }
        }
    }
    for l in loans {
        if let Some(r) = l.reborrow_base {
            edges.push((l.holder.0 as usize, r.0 as usize));
        }
        // borrowing a reference-typed local inherits its loans, or a re-slice reads a freed base
        if local_is_ref(body, l.place.local) {
            edges.push((l.holder.0 as usize, l.place.local.0 as usize));
        }
    }

    // pass-2 call-site edges: a returned reference inherits the loan of the argument it borrows,
    for block in &body.blocks {
        for stmt in &block.stmts {
            let BirStmtKind::Assign {
                dest,
                rvalue: BirRvalue::Call { callee, args, .. },
            } = &stmt.kind
            else {
                continue;
            };
            if !dest.proj.is_empty() {
                continue;
            }
            match callee.as_ref().and_then(|name| summaries.get(name)) {
                Some(SummaryEntry::Unique(ro)) if !ro.escapes_local => {
                    // precise: only the arguments the summary marks as borrowed
                    for (i, arg) in args.iter().enumerate() {
                        if ro.params.get(i).copied().unwrap_or(false) {
                            if let BirOperand::Copy(p) | BirOperand::Move(p) = arg {
                                edges.push((dest.local.0 as usize, p.local.0 as usize));
                            }
                        }
                    }
                }
                _ => {
                    // none / ambiguous / a rejected (escapes_local) callee: over-approximate by
                    for arg in args {
                        if let BirOperand::Copy(p) | BirOperand::Move(p) = arg {
                            if local_is_ref(body, p.local) {
                                edges.push((dest.local.0 as usize, p.local.0 as usize));
                            }
                        }
                    }
                }
            }
        }
    }

    let mut changed = true;
    while changed {
        changed = false;
        for &(dst, src) in &edges {
            if dst >= n || src >= n || dst == src {
                continue;
            }
            let src_ids: Vec<u32> = holds[src].iter().copied().collect();
            for id in src_ids {
                if holds[dst].insert(id) {
                    changed = true;
                }
            }
        }
    }
    holds
}

struct Liveness {
    live_before: Vec<Vec<HashSet<BirLocalId>>>,
    live_at_term: Vec<HashSet<BirLocalId>>,
}

fn successors(term: &BirTerminator) -> Vec<BirBlockId> {
    match term {
        BirTerminator::Goto(t) => vec![*t],
        BirTerminator::Branch { targets, .. } => targets.clone(),
        BirTerminator::Return(_) | BirTerminator::Unreachable => Vec::new(),
    }
}

fn operand_use(op: &BirOperand, out: &mut Vec<BirLocalId>) {
    match op {
        BirOperand::Copy(p) | BirOperand::Move(p) => out.push(p.local),
        BirOperand::Const => {}
    }
}

fn rvalue_reads(rv: &BirRvalue, out: &mut Vec<BirLocalId>) {
    match rv {
        BirRvalue::Use(o) | BirRvalue::UnOp(o) => operand_use(o, out),
        BirRvalue::BinOp(a, b) => {
            operand_use(a, out);
            operand_use(b, out);
        }
        BirRvalue::Aggregate(v) | BirRvalue::Call { args: v, .. } => {
            for o in v {
                operand_use(o, out);
            }
        }
        BirRvalue::Ref { place, .. } | BirRvalue::Reborrow { place, .. } => out.push(place.local),
    }
}

fn stmt_uses(stmt: &BirStmt, out: &mut Vec<BirLocalId>) {
    if let BirStmtKind::Assign { dest, rvalue } = &stmt.kind {
        rvalue_reads(rvalue, out);
        if !dest.proj.is_empty() {
            out.push(dest.local);
        }
    }
}

fn term_uses(term: &BirTerminator, out: &mut Vec<BirLocalId>) {
    match term {
        BirTerminator::Branch { discr, .. } => operand_use(discr, out),
        BirTerminator::Return(Some(o)) => operand_use(o, out),
        _ => {}
    }
}

fn stmt_def(stmt: &BirStmt) -> Option<BirLocalId> {
    if let BirStmtKind::Assign { dest, .. } = &stmt.kind {
        if dest.proj.is_empty() {
            return Some(dest.local);
        }
    }
    None
}

fn transfer_block(block: &BirBlock, live_out: &HashSet<BirLocalId>) -> HashSet<BirLocalId> {
    let mut live = live_out.clone();
    let mut tu = Vec::new();
    term_uses(&block.term, &mut tu);
    for u in tu {
        live.insert(u);
    }
    for stmt in block.stmts.iter().rev() {
        if let Some(d) = stmt_def(stmt) {
            live.remove(&d);
        }
        let mut us = Vec::new();
        stmt_uses(stmt, &mut us);
        for u in us {
            live.insert(u);
        }
    }
    live
}

fn compute_liveness(body: &BirBody) -> Liveness {
    let nblocks = body.blocks.len();
    let mut index: HashMap<u32, usize> = HashMap::new();
    for (i, b) in body.blocks.iter().enumerate() {
        index.insert(b.id.0, i);
    }
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); nblocks];
    for (i, b) in body.blocks.iter().enumerate() {
        for s in successors(&b.term) {
            if let Some(&j) = index.get(&s.0) {
                preds[j].push(i);
            }
        }
    }

    let mut live_in: Vec<HashSet<BirLocalId>> = vec![HashSet::new(); nblocks];
    let mut live_out: Vec<HashSet<BirLocalId>> = vec![HashSet::new(); nblocks];

    let mut worklist: Vec<usize> = (0..nblocks).collect();
    while let Some(bi) = worklist.pop() {
        let block = &body.blocks[bi];
        let mut out: HashSet<BirLocalId> = HashSet::new();
        for s in successors(&block.term) {
            if let Some(&j) = index.get(&s.0) {
                out.extend(live_in[j].iter().copied());
            }
        }
        let new_in = transfer_block(block, &out);
        live_out[bi] = out;
        if live_in[bi] != new_in {
            live_in[bi] = new_in;
            for &p in &preds[bi] {
                worklist.push(p);
            }
        }
    }

    let mut live_before: Vec<Vec<HashSet<BirLocalId>>> = Vec::with_capacity(nblocks);
    let mut live_at_term: Vec<HashSet<BirLocalId>> = Vec::with_capacity(nblocks);
    for bi in 0..nblocks {
        let block = &body.blocks[bi];
        let mut cur = live_out[bi].clone();
        let mut tu = Vec::new();
        term_uses(&block.term, &mut tu);
        for u in tu {
            cur.insert(u);
        }
        live_at_term.push(cur.clone());
        let nstmts = block.stmts.len();
        let mut per: Vec<HashSet<BirLocalId>> = vec![HashSet::new(); nstmts];
        for i in (0..nstmts).rev() {
            if let Some(d) = stmt_def(&block.stmts[i]) {
                cur.remove(&d);
            }
            let mut us = Vec::new();
            stmt_uses(&block.stmts[i], &mut us);
            for u in us {
                cur.insert(u);
            }
            per[i] = cur.clone();
        }
        live_before.push(per);
    }

    Liveness {
        live_before,
        live_at_term,
    }
}

fn places_conflict(a: &BirPlace, b: &BirPlace) -> bool {
    if a.local != b.local {
        return false;
    }
    let mut ai = a.proj.iter();
    let mut bi = b.proj.iter();
    loop {
        match (ai.next(), bi.next()) {
            (Some(pa), Some(pb)) => match (pa, pb) {
                (BirProjection::Field(x), BirProjection::Field(y)) => {
                    if x != y {
                        return false;
                    }
                }
                (BirProjection::Index, BirProjection::Index) => return true,
                (BirProjection::Deref, BirProjection::Deref) => {}
                // e.g. field vs index cannot alias
                _ => return false,
            },
            _ => return true,
        }
    }
}

// accept iff the loan is shared and the event is a shared read or borrow, else reject
fn compatible(access: Access, kind: LoanKind) -> bool {
    match kind {
        LoanKind::Mut => false,
        LoanKind::Shared => matches!(access, Access::Read | Access::BorrowShared),
    }
}

fn push_operand_event<'a>(o: &'a BirOperand, out: &mut Vec<(&'a BirPlace, Access)>) {
    match o {
        BirOperand::Copy(p) => out.push((p, Access::Read)),
        BirOperand::Move(p) => out.push((p, Access::Move)),
        BirOperand::Const => {}
    }
}

fn rvalue_operand_events<'a>(rv: &'a BirRvalue, out: &mut Vec<(&'a BirPlace, Access)>) {
    match rv {
        BirRvalue::Use(o) | BirRvalue::UnOp(o) => push_operand_event(o, out),
        BirRvalue::BinOp(a, b) => {
            push_operand_event(a, out);
            push_operand_event(b, out);
        }
        BirRvalue::Aggregate(v) | BirRvalue::Call { args: v, .. } => {
            for o in v {
                push_operand_event(o, out);
            }
        }
        // a ref is a borrow event, handled by the caller, never also a read of its place
        BirRvalue::Ref { .. } | BirRvalue::Reborrow { .. } => {}
    }
}

fn stmt_events(stmt: &BirStmt) -> Vec<(&BirPlace, Access)> {
    let mut out = Vec::new();
    if let BirStmtKind::Assign { dest, rvalue } = &stmt.kind {
        out.push((dest, Access::Write));
        match rvalue {
            BirRvalue::Ref { place, mutable } | BirRvalue::Reborrow { place, mutable } => {
                let access = if *mutable {
                    Access::BorrowMut
                } else {
                    Access::BorrowShared
                };
                out.push((place, access));
            }
            _ => rvalue_operand_events(rvalue, &mut out),
        }
    }
    out
}

fn term_events(term: &BirTerminator) -> Vec<(&BirPlace, Access)> {
    let mut out = Vec::new();
    match term {
        BirTerminator::Branch { discr, .. } => push_operand_event(discr, &mut out),
        BirTerminator::Return(Some(o)) => push_operand_event(o, &mut out),
        _ => {}
    }
    out
}

fn created_loan_id(loans: &[Loan], block: BirBlockId, index: usize) -> Option<u32> {
    loans
        .iter()
        .find(|l| l.block == block && l.index == index)
        .map(|l| l.id.0)
}

fn check_conflicts(
    body: &BirBody,
    loans: &[Loan],
    holds: &[HashSet<u32>],
    live: &Liveness,
    errors: &mut Vec<BirDiagnostic>,
) {
    for (bi, block) in body.blocks.iter().enumerate() {
        for (i, stmt) in block.stmts.iter().enumerate() {
            let created = created_loan_id(loans, block.id, i);
            let events = stmt_events(stmt);
            check_events(
                body,
                loans,
                holds,
                live,
                &live.live_before[bi][i],
                created,
                &events,
                stmt.span,
                errors,
            );
        }
        // the terminator only reads; a borrow can never be created in a terminator
        let events = term_events(&block.term);
        check_events(
            body,
            loans,
            holds,
            live,
            &live.live_at_term[bi],
            None,
            &events,
            block.term_span,
            errors,
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn check_events(
    body: &BirBody,
    loans: &[Loan],
    holds: &[HashSet<u32>],
    live: &Liveness,
    live_set: &HashSet<BirLocalId>,
    created: Option<u32>,
    events: &[(&BirPlace, Access)],
    event_span: Span,
    errors: &mut Vec<BirDiagnostic>,
) {
    if events.is_empty() {
        return;
    }
    let mut live_loan_ids: HashSet<u32> = HashSet::new();
    for l in live_set {
        if let Some(h) = holds.get(l.0 as usize) {
            live_loan_ids.extend(h.iter().copied());
        }
    }
    if live_loan_ids.is_empty() {
        return;
    }
    for (place, access) in events {
        for l in loans {
            if !live_loan_ids.contains(&l.id.0) {
                continue;
            }
            // a borrow event does not conflict with the loan it is itself creating
            if Some(l.id.0) == created && matches!(access, Access::BorrowShared | Access::BorrowMut)
            {
                continue;
            }
            if !places_conflict(place, &l.place) {
                continue;
            }
            if compatible(*access, l.kind) {
                continue;
            }
            let (code, message) = conflict_message(body, place, *access, l);
            let mut diag = BirDiagnostic::new(code, "[borrow]", event_span, message)
                .with_secondary(l.span, "borrow created here".to_string());
            if let Some(last) = loan_last_use(l, holds, live, body) {
                diag = diag.with_secondary(last, "borrow last used here".to_string());
            }
            errors.push(diag);
        }
    }
}

fn loan_last_use(
    loan: &Loan,
    holds: &[HashSet<u32>],
    live: &Liveness,
    body: &BirBody,
) -> Option<Span> {
    let mut holders: HashSet<BirLocalId> = HashSet::new();
    for (local, ids) in holds.iter().enumerate() {
        if ids.contains(&loan.id.0) {
            holders.insert(BirLocalId(local as u32));
        }
    }
    let mut best: Option<Span> = None;
    for (bi, block) in body.blocks.iter().enumerate() {
        for (i, stmt) in block.stmts.iter().enumerate() {
            if live.live_before[bi][i].iter().any(|l| holders.contains(l)) {
                best = pick_later(best, stmt.span);
            }
        }
        if live.live_at_term[bi].iter().any(|l| holders.contains(l)) {
            best = pick_later(best, block.term_span);
        }
    }
    best
}

fn pick_later(best: Option<Span>, cand: Span) -> Option<Span> {
    match best {
        Some(b) if (b.start, b.end) >= (cand.start, cand.end) => Some(b),
        _ => Some(cand),
    }
}

// a borrow that outlives its referent

// regardless of how the loan was acquired (direct &x, whole-local copy, or reborrow inheritance).
fn check_scope_deaths(
    body: &BirBody,
    loans: &[Loan],
    holds: &[HashSet<u32>],
    live: &Liveness,
    errors: &mut Vec<BirDiagnostic>,
) {
    let mut index: HashMap<u32, usize> = HashMap::new();
    for (i, b) in body.blocks.iter().enumerate() {
        index.insert(b.id.0, i);
    }
    for sd in &body.scope_deaths {
        let Some(&bi) = index.get(&sd.block.0) else {
            continue;
        };
        let live_set = if sd.index < live.live_before[bi].len() {
            &live.live_before[bi][sd.index]
        } else {
            &live.live_at_term[bi]
        };
        let mut live_loan_ids: HashSet<u32> = HashSet::new();
        for l in live_set {
            if let Some(h) = holds.get(l.0 as usize) {
                live_loan_ids.extend(h.iter().copied());
            }
        }
        if live_loan_ids.is_empty() {
            continue;
        }
        for x in &sd.locals {
            if let Some(loan) = loans
                .iter()
                .find(|l| l.place.local == *x && live_loan_ids.contains(&l.id.0))
            {
                let name = local_name(body, *x);
                let primary = loan_last_use(loan, holds, live, body).unwrap_or(loan.span);
                let decl = body.locals[x.0 as usize].decl_span;
                errors.push(
                    BirDiagnostic::new("E0722", "[escape]", primary, scope_death_message(body, *x))
                        .with_secondary(decl, format!("`{name}` declared here"))
                        .with_secondary(sd.scope_span, format!("scope of `{name}` ends here")),
                );
            }
        }
    }
}

fn local_name(body: &BirBody, x: BirLocalId) -> String {
    body.locals
        .get(x.0 as usize)
        .and_then(|l| l.name.clone())
        .unwrap_or_else(|| "value".to_string())
}

fn scope_death_message(body: &BirBody, x: BirLocalId) -> String {
    let name = body
        .locals
        .get(x.0 as usize)
        .and_then(|l| l.name.clone())
        .unwrap_or_else(|| "value".to_string());
    format!(
        "[escape] `{name}` does not live long enough; it is borrowed and the borrow is used \
         after `{name}`'s scope ends"
    )
}

fn conflict_message(
    body: &BirBody,
    place: &BirPlace,
    access: Access,
    _loan: &Loan,
) -> (&'static str, String) {
    let name = render_place(body, place);
    match access {
        Access::Write => (
            "E0711",
            format!("[borrow] cannot write to `{name}` while it is borrowed"),
        ),
        Access::Move => (
            "E0712",
            format!("[borrow] cannot move `{name}` while it is borrowed"),
        ),
        Access::BorrowMut => (
            "E0713",
            format!("[borrow] cannot borrow `{name}` as mutable while it is already borrowed"),
        ),
        Access::BorrowShared => (
            "E0713",
            format!(
                "[borrow] cannot borrow `{name}` as shared while it is already mutably borrowed"
            ),
        ),
        Access::Read => (
            "E0713",
            format!("[borrow] cannot use `{name}` while it is mutably borrowed"),
        ),
    }
}

fn render_place(body: &BirBody, place: &BirPlace) -> String {
    let base = body
        .locals
        .get(place.local.0 as usize)
        .and_then(|l| l.name.clone())
        .unwrap_or_else(|| "value".to_string());
    let mut derefs = 0usize;
    let mut suffix = String::new();
    for p in &place.proj {
        match p {
            BirProjection::Field(f) => {
                suffix.push('.');
                suffix.push_str(f);
            }
            BirProjection::Index => suffix.push_str("[..]"),
            BirProjection::Deref => derefs += 1,
        }
    }
    let mut s = String::new();
    for _ in 0..derefs {
        s.push('*');
    }
    s.push_str(&base);
    s.push_str(&suffix);
    s
}

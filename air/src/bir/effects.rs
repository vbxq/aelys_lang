use std::collections::{HashMap, HashSet};

use aelys_sema::{
    InferType, ResultAssertOnErr, TypeTable, TypedExpr, TypedExprKind, TypedFmtStringPart,
    TypedParam, TypedStmt, TypedStmtKind,
};
use aelys_syntax::{BinaryOp, Span};

use super::category::{Category, category};
use super::{BirBody, BirProgram, BirRvalue, BirStmtKind};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Effect {
    Managed,
    Alloc,
    Panic,
    Unwind,
    Block,
    Io,
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub struct EffectSet(u8);

impl EffectSet {
    pub const EMPTY: EffectSet = EffectSet(0);

    pub fn contains(self, e: Effect) -> bool {
        self.0 & (1 << e as u8) != 0
    }

    pub fn insert(&mut self, e: Effect) {
        self.0 |= 1 << e as u8;
    }

    pub fn with(mut self, e: Effect) -> EffectSet {
        self.insert(e);
        self
    }

    pub fn union(self, other: EffectSet) -> EffectSet {
        EffectSet(self.0 | other.0)
    }

    pub fn is_subset_of(self, other: EffectSet) -> bool {
        self.0 & other.0 == self.0
    }

    pub fn is_nogc(self) -> bool {
        !self.contains(Effect::Managed)
    }
}

const RANK_INTRINSIC: u8 = 1;
const RANK_LITERAL: u8 = 2;
const RANK_TYPE: u8 = 3;

#[derive(Default)]
struct Witness {
    rank: u8,
    site: Option<(Span, String)>,
}

impl Witness {
    fn record(&mut self, rank: u8, span: Span, name: String) {
        if self.rank == 0 || rank < self.rank {
            self.rank = rank;
            self.site = Some((span, name));
        }
    }
}

pub fn intrinsic_effects(
    body: &[TypedStmt],
    params: &[TypedParam],
    tt: &TypeTable,
) -> (EffectSet, Option<(Span, String)>) {
    let mut set = EffectSet::EMPTY;
    let mut w = Witness::default();
    for p in params {
        if category(&p.ty, tt) == Category::Managed {
            set.insert(Effect::Managed);
            w.record(
                RANK_TYPE,
                p.span,
                format!("the managed parameter `{}`", p.name),
            );
        }
    }
    for stmt in body {
        walk_stmt(stmt, tt, &mut set, &mut w);
    }
    (set, w.site)
}

fn alloc_managed(set: &mut EffectSet) {
    set.insert(Effect::Alloc);
    set.insert(Effect::Managed);
}

fn is_managed_alloc_variant(enum_name: &str, variant: &str) -> bool {
    matches!(
        (enum_name, variant),
        ("Rc", "new")
            | ("Vec", "new")
            | ("Vec", "push")
            | ("Vec", "with_capacity")
            | ("Vec", "reserve")
    )
}

fn walk_stmt(stmt: &TypedStmt, tt: &TypeTable, set: &mut EffectSet, w: &mut Witness) {
    match &stmt.kind {
        TypedStmtKind::Expression(e) => walk_expr(e, tt, set, w),
        TypedStmtKind::Let {
            name,
            initializer,
            var_type,
            ..
        } => {
            if category(var_type, tt) == Category::Managed {
                set.insert(Effect::Managed);
                w.record(
                    RANK_TYPE,
                    stmt.span,
                    format!("the managed local `{}`", name),
                );
            }
            walk_expr(initializer, tt, set, w);
        }
        TypedStmtKind::Block(stmts) => {
            for s in stmts {
                walk_stmt(s, tt, set, w);
            }
        }
        TypedStmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            walk_expr(condition, tt, set, w);
            walk_stmt(then_branch, tt, set, w);
            if let Some(e) = else_branch {
                walk_stmt(e, tt, set, w);
            }
        }
        TypedStmtKind::While { condition, body } => {
            walk_expr(condition, tt, set, w);
            walk_stmt(body, tt, set, w);
        }
        TypedStmtKind::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            walk_expr(start, tt, set, w);
            walk_expr(end, tt, set, w);
            if let Some(s) = step.as_ref().as_ref() {
                walk_expr(s, tt, set, w);
            }
            walk_stmt(body, tt, set, w);
        }
        TypedStmtKind::ForEach { iterable, body, .. } => {
            walk_expr(iterable, tt, set, w);
            walk_stmt(body, tt, set, w);
        }
        TypedStmtKind::Return(val) => {
            if let Some(e) = val {
                walk_expr(e, tt, set, w);
            }
        }
        TypedStmtKind::Break | TypedStmtKind::Continue => {}
        TypedStmtKind::Function(_)
        | TypedStmtKind::Needs(_)
        | TypedStmtKind::StructDecl { .. }
        | TypedStmtKind::EnumDecl { .. } => {}
    }
}

fn walk_expr(expr: &TypedExpr, tt: &TypeTable, set: &mut EffectSet, w: &mut Witness) {
    if category(&expr.ty, tt) == Category::Managed {
        set.insert(Effect::Managed);
        let name = match &expr.kind {
            TypedExprKind::Identifier(n) => format!("the managed value `{}`", n),
            _ => "a managed value".to_string(),
        };
        w.record(RANK_TYPE, expr.span, name);
    }
    match &expr.kind {
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Null
        | TypedExprKind::Identifier(_) => {}

        TypedExprKind::FmtString(parts) => {
            alloc_managed(set);
            w.record(RANK_LITERAL, expr.span, "a format string".to_string());
            for part in parts {
                if let TypedFmtStringPart::Expr(e) = part {
                    walk_expr(e, tt, set, w);
                }
            }
        }

        TypedExprKind::Binary { left, op, right } => {
            match op {
                BinaryOp::Div | BinaryOp::Mod => set.insert(Effect::Panic),
                BinaryOp::Add if left.ty == InferType::String || right.ty == InferType::String => {
                    alloc_managed(set);
                    w.record(RANK_LITERAL, expr.span, "string concatenation".to_string());
                }
                _ => {}
            }
            walk_expr(left, tt, set, w);
            walk_expr(right, tt, set, w);
        }
        TypedExprKind::Unary { operand, .. } => walk_expr(operand, tt, set, w),
        TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
            walk_expr(left, tt, set, w);
            walk_expr(right, tt, set, w);
        }

        TypedExprKind::Call { callee, args } => {
            walk_expr(callee, tt, set, w);
            for a in args {
                walk_expr(a, tt, set, w);
            }
        }

        TypedExprKind::Assign { value, .. } => walk_expr(value, tt, set, w),
        TypedExprKind::Grouping(inner) => walk_expr(inner, tt, set, w),

        TypedExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            walk_expr(condition, tt, set, w);
            walk_expr(then_branch, tt, set, w);
            walk_expr(else_branch, tt, set, w);
        }

        TypedExprKind::LambdaInner { captures, .. } => {
            if !captures.is_empty() {
                alloc_managed(set);
                w.record(RANK_LITERAL, expr.span, "a capturing closure".to_string());
            }
        }
        TypedExprKind::Lambda(inner) => walk_expr(inner, tt, set, w),

        TypedExprKind::Member { object, .. } => walk_expr(object, tt, set, w),

        TypedExprKind::ArrayLiteral { elements } => {
            for e in elements {
                walk_expr(e, tt, set, w);
            }
        }
        TypedExprKind::ArraySized { size, fill_value } => {
            walk_expr(size, tt, set, w);
            if let Some(fv) = fill_value {
                walk_expr(fv, tt, set, w);
            }
        }
        TypedExprKind::VecLiteral { elements, .. } => {
            alloc_managed(set);
            w.record(RANK_LITERAL, expr.span, "a vec literal".to_string());
            for e in elements {
                walk_expr(e, tt, set, w);
            }
        }

        TypedExprKind::Index { object, index } => {
            set.insert(Effect::Panic);
            walk_expr(object, tt, set, w);
            walk_expr(index, tt, set, w);
        }
        TypedExprKind::IndexAssign {
            object,
            index,
            value,
        } => {
            set.insert(Effect::Panic);
            walk_expr(object, tt, set, w);
            walk_expr(index, tt, set, w);
            walk_expr(value, tt, set, w);
        }
        TypedExprKind::FieldAssign { object, value, .. } => {
            walk_expr(object, tt, set, w);
            walk_expr(value, tt, set, w);
        }
        TypedExprKind::Range { start, end, .. } => {
            if let Some(s) = start {
                walk_expr(s, tt, set, w);
            }
            if let Some(e) = end {
                walk_expr(e, tt, set, w);
            }
        }
        TypedExprKind::Slice { object, range } => {
            set.insert(Effect::Panic);
            walk_expr(object, tt, set, w);
            walk_expr(range, tt, set, w);
        }
        TypedExprKind::Reference { operand, .. } => walk_expr(operand, tt, set, w),
        TypedExprKind::Deref(inner) => walk_expr(inner, tt, set, w),
        TypedExprKind::DerefAssign { target, value } => {
            walk_expr(target, tt, set, w);
            walk_expr(value, tt, set, w);
        }
        TypedExprKind::StructLiteral { fields, .. } => {
            for (_, v) in fields {
                walk_expr(v, tt, set, w);
            }
        }
        TypedExprKind::Cast { expr: inner, .. } => walk_expr(inner, tt, set, w),

        TypedExprKind::EnumVariant {
            enum_name,
            variant,
            args,
            ..
        } => {
            if is_managed_alloc_variant(enum_name, variant) {
                alloc_managed(set);
                w.record(
                    RANK_INTRINSIC,
                    expr.span,
                    format!("{}::{}", enum_name, variant),
                );
            }
            for a in args {
                walk_expr(a, tt, set, w);
            }
        }

        TypedExprKind::Match { scrutinee, arms } => {
            walk_expr(scrutinee, tt, set, w);
            for arm in arms {
                walk_expr(&arm.body, tt, set, w);
            }
        }

        TypedExprKind::ResultAssert {
            scrutinee, on_err, ..
        } => {
            if let ResultAssertOnErr::Panic(_) = on_err {
                set.insert(Effect::Panic);
            }
            walk_expr(scrutinee, tt, set, w);
        }

        TypedExprKind::Block { stmts, tail } => {
            for s in stmts {
                walk_stmt(s, tt, set, w);
            }
            walk_expr(tail, tt, set, w);
        }
    }
}

const TOP: EffectSet = EffectSet(
    (1 << Effect::Managed as u8) | (1 << Effect::Alloc as u8) | (1 << Effect::Panic as u8),
);

pub fn effect_summaries(bir: &BirProgram) -> HashMap<String, EffectSet> {
    let mut seed: HashMap<String, EffectSet> = HashMap::new();
    let mut edges: HashMap<String, Vec<(HashSet<String>, bool)>> = HashMap::new();
    for body in &bir.bodies {
        let s = seed.entry(body.name.clone()).or_insert(EffectSet::EMPTY);
        *s = s.union(body.intrinsic_effects);

        let mut direct = HashSet::new();
        let mut has_indirect = false;
        for block in &body.blocks {
            for stmt in &block.stmts {
                if let BirStmtKind::Assign {
                    rvalue:
                        BirRvalue::Call {
                            callee,
                            indirect_nogc,
                            ..
                        },
                    ..
                } = &stmt.kind
                {
                    match callee {
                        Some(n) => {
                            direct.insert(n.clone());
                        }
                        None if *indirect_nogc => {}
                        None => has_indirect = true,
                    }
                }
            }
        }
        edges
            .entry(body.name.clone())
            .or_default()
            .push((direct, has_indirect));
    }

    // monotone round-robin fixpoint (same while-changed idiom as origins.rs, lifted to the call
    let mut eff = seed.clone();
    let mut changed = true;
    while changed {
        changed = false;
        for (name, bodies) in &edges {
            let mut new = seed[name];
            for (direct, has_indirect) in bodies {
                if *has_indirect {
                    new = new.union(TOP);
                }
                for callee in direct {
                    new = new.union(eff.get(callee).copied().unwrap_or(TOP));
                }
            }
            if new != eff[name] {
                eff.insert(name.clone(), new);
                changed = true;
            }
        }
    }

    eff
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub enum StepKind {
    Root,
    Callee,
    Operation,
    Ambiguous,
    Indirect,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Step {
    pub name: String,
    pub span: Option<Span>,
    pub kind: StepKind,
}

// the e0727 witness, rebuilt after the fixpoint because the fixpoint iterates hash containers
pub fn managed_chain<'a>(
    bir: &'a BirProgram,
    eff: &HashMap<String, EffectSet>,
    start: &'a BirBody,
) -> Vec<Step> {
    let mut steps = vec![Step {
        name: start.name.clone(),
        span: Some(start.span),
        kind: StepKind::Root,
    }];
    let mut frames: Vec<(usize, &'a BirBody)> = vec![(0, start)];
    let mut visited: HashSet<String> = HashSet::new();
    visited.insert(start.name.clone());
    let mut body = start;

    loop {
        let Some((span, callee)) = next_managed_call(body, eff, &visited) else {
            return close_on_witness(steps, &frames, None);
        };
        let Some(name) = callee else {
            steps.push(Step {
                name: "<indirect call>".to_string(),
                span: Some(span),
                kind: StepKind::Indirect,
            });
            return steps;
        };
        let mut matching = bir.bodies.iter().filter(|b| b.name == name);
        match (matching.next(), matching.next().is_some()) {
            (Some(next), false) => {
                steps.push(Step {
                    name: name.to_string(),
                    span: Some(span),
                    kind: StepKind::Callee,
                });
                frames.push((steps.len() - 1, next));
                visited.insert(name.to_string());
                body = next;
            }
            (Some(_), true) => return close_on_witness(steps, &frames, Some(name)),
            (None, _) => return close_on_witness(steps, &frames, None),
        }
    }
}

fn close_on_witness(
    mut steps: Vec<Step>,
    frames: &[(usize, &BirBody)],
    ambiguous: Option<&str>,
) -> Vec<Step> {
    for (index, body) in frames.iter().rev() {
        let Some((span, name)) = &body.managed_witness else {
            continue;
        };
        steps.truncate(index + 1);
        steps.push(Step {
            name: name.clone(),
            span: Some(*span),
            kind: StepKind::Operation,
        });
        return steps;
    }
    if let Some(name) = ambiguous {
        steps.push(Step {
            name: name.to_string(),
            span: None,
            kind: StepKind::Ambiguous,
        });
    }
    steps
}

fn next_managed_call<'a>(
    body: &'a BirBody,
    eff: &HashMap<String, EffectSet>,
    visited: &HashSet<String>,
) -> Option<(Span, Option<&'a str>)> {
    let mut best: Option<(Span, Option<&'a str>)> = None;
    for block in &body.blocks {
        for stmt in &block.stmts {
            let BirStmtKind::Assign {
                rvalue:
                    BirRvalue::Call {
                        callee,
                        indirect_nogc,
                        ..
                    },
                ..
            } = &stmt.kind
            else {
                continue;
            };
            let hop = match callee {
                Some(n) => {
                    if visited.contains(n) {
                        continue;
                    }
                    if !eff.get(n).copied().unwrap_or(TOP).contains(Effect::Managed) {
                        continue;
                    }
                    Some(n.as_str())
                }
                None if *indirect_nogc => continue,
                None => None,
            };
            if best.map_or(true, |(s, _)| {
                (stmt.span.start, stmt.span.end) < (s.start, s.end)
            }) {
                best = Some((stmt.span, hop));
            }
        }
    }
    best
}

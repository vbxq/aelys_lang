use aelys_sema::{
    InferType, TypedExpr, TypedExprKind, TypedFmtStringPart, TypedParam, TypedPattern, TypedStmt,
    TypedStmtKind,
};
use std::collections::{HashMap, HashSet};

pub(super) fn is_view(ty: &InferType) -> bool {
    matches!(ty, InferType::Slice { .. } | InferType::Ref { .. })
}

pub(super) fn key<T>(x: &T) -> usize {
    x as *const T as usize
}

pub(super) const MAX_WIDTH: usize = 8;

#[derive(Clone, Default, PartialEq, Eq, Debug)]
pub(crate) struct Width(Vec<(crate::AirType, usize)>);

pub(super) fn rc_slot_ty() -> crate::AirType {
    crate::AirType::Ptr(Box::new(crate::AirType::Void))
}

impl Width {
    pub(super) fn of(ty: crate::AirType, n: usize) -> Width {
        Width(if n == 0 { Vec::new() } else { vec![(ty, n)] })
    }

    fn count(&self, ty: &crate::AirType) -> usize {
        self.0.iter().find(|(t, _)| t == ty).map_or(0, |(_, n)| *n)
    }

    fn merged(&self, o: &Width, f: impl Fn(usize, usize) -> usize) -> Width {
        let mut out = self.0.clone();
        for (t, n) in &o.0 {
            match out.iter_mut().find(|(u, _)| u == t) {
                Some(slot) => slot.1 = f(slot.1, *n),
                None => out.push((t.clone(), f(0, *n))),
            }
        }
        Width(out)
    }

    fn plus(&self, o: &Width) -> Width {
        self.merged(o, |a, b| (a + b).min(MAX_WIDTH + 1))
    }

    fn max(&self, o: &Width) -> Width {
        self.merged(o, usize::max)
    }

    pub(super) fn rc(&self) -> usize {
        self.count(&rc_slot_ty())
    }

    pub(super) fn text(&self) -> usize {
        self.0
            .iter()
            .filter(|(t, _)| *t != rc_slot_ty())
            .map(|(_, n)| *n)
            .sum()
    }

    pub(super) fn over(&self) -> bool {
        self.rc() > MAX_WIDTH || self.text() > MAX_WIDTH
    }

    pub(super) fn is_empty(&self) -> bool {
        self.0.iter().all(|(_, n)| *n == 0)
    }

    pub(super) fn slot_types(&self) -> Vec<crate::AirType> {
        self.0
            .iter()
            .flat_map(|(t, n)| std::iter::repeat_n(t.clone(), *n))
            .collect()
    }
}

#[derive(Default)]
pub(crate) struct ViewOwners {
    pub width: HashMap<usize, Width>,
    pub ident_width: HashMap<usize, Width>,
    pub str_nodes: HashSet<usize>,
    pub leaf_nodes: HashMap<usize, (Result<Width, &'static str>, u32)>,
}

struct Pass<'f> {
    scopes: Vec<HashMap<String, Option<usize>>>,
    edges: Vec<(usize, Vec<SrcRef>)>,
    ident_decl: HashMap<usize, usize>,
    str_nodes: HashSet<usize>,
    leaf_nodes: HashMap<usize, (Result<Width, &'static str>, u32)>,
    leaves: &'f dyn Fn(&InferType) -> Result<Width, &'static str>,
}

#[derive(Clone)]
enum SrcRef {
    Slots(Width),
    Decl(usize),
}

#[derive(Clone, Copy)]
enum Sig {
    Plain,
    BytesOf,
    RefTarget(usize),
}

#[derive(Clone)]
pub(super) enum Src {
    Rc,
    Text,
    Leaves(Width, usize),
    Ident(Width),
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum CapKind {
    Rc,
    Text,
    Leaves(usize),
    Ident,
}

impl Src {
    fn kind(&self) -> CapKind {
        match self {
            Src::Rc => CapKind::Rc,
            Src::Text => CapKind::Text,
            Src::Leaves(_, call) => CapKind::Leaves(*call),
            Src::Ident(_) => CapKind::Ident,
        }
    }

    fn width(&self) -> Width {
        match self {
            Src::Rc => Width::of(rc_slot_ty(), 1),
            Src::Text => Width::of(crate::AirType::Str, 1),
            Src::Leaves(w, _) | Src::Ident(w) => w.clone(),
        }
    }
}

pub(super) fn is_byte_view(ty: &InferType) -> bool {
    matches!(ty, InferType::Slice { elem, .. } if matches!(elem.as_ref(), InferType::U8))
}

pub(super) fn sources(expr: &TypedExpr, owners: &ViewOwners) -> Vec<(usize, Src)> {
    let mut out = Vec::new();
    collect(expr, &mut |e, sig| match sig {
        Sig::Plain => {
            if matches!(e.ty, InferType::Rc(_)) {
                out.push((key(e), Src::Rc));
            } else if let Some(w) = owners.ident_width.get(&key(e))
                && !w.over()
                && !w.is_empty()
            {
                out.push((key(e), Src::Ident(w.clone())));
            }
        }
        Sig::BytesOf => {
            if owners.str_nodes.contains(&key(e)) {
                out.push((key(e), Src::Text));
            }
        }
        Sig::RefTarget(call) => {
            if let Some((Ok(w), _)) = owners.leaf_nodes.get(&key(e))
                && !w.is_empty()
            {
                out.push((key(e), Src::Leaves(w.clone(), call)));
            }
        }
    });
    out
}

pub(super) fn frame_held(expr: &TypedExpr, is_local: &dyn Fn(&str) -> bool) -> bool {
    match &expr.kind {
        TypedExprKind::Grouping(inner) => frame_held(inner, is_local),
        TypedExprKind::Identifier(name) => is_local(name) && !is_view(&expr.ty),
        TypedExprKind::Member { object, .. } | TypedExprKind::Index { object, .. } => {
            matches!(
                object.ty,
                InferType::Struct(_) | InferType::Enum(..) | InferType::Array(..)
            ) && frame_held(object, is_local)
        }
        _ => false,
    }
}

pub(super) fn target_held(arg: &TypedExpr, is_local: &dyn Fn(&str) -> bool) -> bool {
    match &arg.kind {
        TypedExprKind::Grouping(inner) => target_held(inner, is_local),
        TypedExprKind::Reference { operand, .. } => frame_held(operand, is_local),
        TypedExprKind::Slice { object, .. } => frame_held(object, is_local),
        _ => false,
    }
}

fn collect<'e>(expr: &'e TypedExpr, f: &mut dyn FnMut(&'e TypedExpr, Sig)) {
    if matches!(expr.ty, InferType::Rc(_)) {
        f(expr, Sig::Plain);
        return;
    }
    collect_place(expr, f);
}

fn collect_place<'e>(expr: &'e TypedExpr, f: &mut dyn FnMut(&'e TypedExpr, Sig)) {
    match &expr.kind {
        TypedExprKind::Grouping(inner) | TypedExprKind::Deref(inner) => collect(inner, f),
        TypedExprKind::Member { object, member }
            if member == "bytes" && matches!(object.ty, InferType::String) =>
        {
            f(object, Sig::BytesOf);
            collect(object, f);
        }
        TypedExprKind::Slice { object, .. }
        | TypedExprKind::Index { object, .. }
        | TypedExprKind::Member { object, .. } => collect(object, f),
        TypedExprKind::Reference { operand, .. } => collect_place(operand, f),
        TypedExprKind::EnumVariant {
            enum_name,
            variant,
            args,
            ..
        } if enum_name == "Rc" && variant == "get" => {
            if let Some(h) = args.first() {
                collect(h, f);
            }
        }
        TypedExprKind::Identifier(_) if !matches!(expr.ty, InferType::Rc(_)) => f(expr, Sig::Plain),
        TypedExprKind::Block { tail, .. } => collect(tail, f),
        TypedExprKind::If {
            then_branch,
            else_branch,
            ..
        } => {
            collect(then_branch, f);
            collect(else_branch, f);
        }
        TypedExprKind::Match { arms, .. } => {
            for arm in arms {
                collect(&arm.body, f);
            }
        }
        TypedExprKind::Call { args, .. } if is_view(&expr.ty) => {
            for a in args {
                if matches!(a.ty, InferType::Rc(_)) {
                    collect(a, f);
                } else if is_view(&a.ty) {
                    if is_byte_view(&expr.ty) {
                        f(a, Sig::RefTarget(key(expr)));
                    }
                    collect(a, f);
                }
            }
        }
        _ => {}
    }
}

pub(super) fn analyze(
    params: &[TypedParam],
    body: &[TypedStmt],
    leaves: &dyn Fn(&InferType) -> Result<Width, &'static str>,
) -> ViewOwners {
    let mut p = Pass {
        scopes: vec![HashMap::new()],
        edges: Vec::new(),
        ident_decl: HashMap::new(),
        str_nodes: HashSet::new(),
        leaf_nodes: HashMap::new(),
        leaves,
    };
    for param in params {
        let decl = is_view(&param.ty).then(|| key(param));
        p.scopes[0].insert(param.name.clone(), decl);
    }
    for s in body {
        p.stmt(s);
    }
    let mut width: HashMap<usize, Width> = HashMap::new();
    loop {
        let mut changed = false;
        for (decl, srcs) in &p.edges {
            let need = srcs.iter().fold(Width::default(), |acc, s| {
                acc.plus(&match s {
                    SrcRef::Slots(w) => w.clone(),
                    SrcRef::Decl(d) => width.get(d).cloned().unwrap_or_default(),
                })
            });
            let have = width.get(decl).cloned().unwrap_or_default();
            let next = have.max(&need);
            if next != have {
                width.insert(*decl, next);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let ident_width = p
        .ident_decl
        .iter()
        .filter_map(|(i, d)| width.get(d).map(|w| (*i, w.clone())))
        .collect();
    ViewOwners {
        width,
        ident_width,
        str_nodes: p.str_nodes,
        leaf_nodes: p.leaf_nodes,
    }
}

impl Pass<'_> {
    fn resolve(&self, name: &str) -> Option<usize> {
        for scope in self.scopes.iter().rev() {
            if let Some(d) = scope.get(name) {
                return *d;
            }
        }
        None
    }

    fn is_local(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains_key(name))
    }

    fn srcs_of(&mut self, rhs: &TypedExpr) -> Vec<SrcRef> {
        let mut found: Vec<(&TypedExpr, Sig)> = Vec::new();
        collect(rhs, &mut |e, sig| found.push((e, sig)));
        let mut out = Vec::new();
        for (e, sig) in found {
            match sig {
                Sig::Plain => {
                    if matches!(e.ty, InferType::Rc(_)) {
                        out.push(SrcRef::Slots(Width::of(rc_slot_ty(), 1)));
                    } else if let Some(d) = self.ident_decl.get(&key(e)) {
                        out.push(SrcRef::Decl(*d));
                    }
                }
                Sig::BytesOf => {
                    if self.str_nodes.contains(&key(e)) {
                        out.push(SrcRef::Slots(Width::of(crate::AirType::Str, 1)));
                    }
                }
                Sig::RefTarget(_) => {
                    if let Some((Ok(w), _)) = self.leaf_nodes.get(&key(e)) {
                        out.push(SrcRef::Slots(w.clone()));
                    }
                }
            }
        }
        out
    }

    fn note_view_roots(&mut self, e: &TypedExpr) {
        if let TypedExprKind::Member { object, member } = &e.kind
            && member == "bytes"
            && matches!(object.ty, InferType::String)
        {
            if !frame_held(object, &|n| self.is_local(n)) {
                self.str_nodes.insert(key(object.as_ref()));
            }
            return;
        }
        let TypedExprKind::Call { args, .. } = &e.kind else {
            return;
        };
        if !is_byte_view(&e.ty) {
            return;
        }
        for a in args {
            if !is_view(&a.ty) || target_held(a, &|n| self.is_local(n)) {
                continue;
            }
            let plan = (self.leaves)(&a.ty);
            if plan.as_ref().map_or(true, |w| !w.is_empty()) {
                self.leaf_nodes.insert(key(a), (plan, a.span.line));
            }
        }
    }

    fn block(&mut self, stmts: &[TypedStmt], tail: Option<&TypedExpr>) {
        self.scopes.push(HashMap::new());
        for s in stmts {
            self.stmt(s);
        }
        if let Some(t) = tail {
            self.expr(t);
        }
        self.scopes.pop();
    }

    fn view_rhs(&mut self, rhs: &TypedExpr) -> Vec<SrcRef> {
        self.expr(rhs);
        self.srcs_of(rhs)
    }

    fn stmt(&mut self, s: &TypedStmt) {
        match &s.kind {
            TypedStmtKind::Expression(e) => self.expr(e),
            TypedStmtKind::Let {
                name,
                initializer,
                var_type,
                ..
            } => {
                if is_view(var_type) {
                    let srcs = self.view_rhs(initializer);
                    self.edges.push((key(s), srcs));
                    self.scopes
                        .last_mut()
                        .unwrap()
                        .insert(name.clone(), Some(key(s)));
                } else {
                    self.expr(initializer);
                    self.scopes.last_mut().unwrap().insert(name.clone(), None);
                }
            }
            TypedStmtKind::Block(stmts) => self.block(stmts, None),
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition);
                self.scoped(then_branch);
                if let Some(e) = else_branch {
                    self.scoped(e);
                }
            }
            TypedStmtKind::While { condition, body } => {
                self.expr(condition);
                self.scoped(body);
            }
            TypedStmtKind::For {
                iterator,
                start,
                end,
                step,
                body,
                ..
            } => {
                self.expr(start);
                self.expr(end);
                if let Some(st) = step.as_ref() {
                    self.expr(st);
                }
                self.scopes.push(HashMap::from([(iterator.clone(), None)]));
                self.stmt(body);
                self.scopes.pop();
            }
            TypedStmtKind::ForEach {
                iterator,
                iterable,
                body,
                ..
            } => {
                self.expr(iterable);
                self.scopes.push(HashMap::from([(iterator.clone(), None)]));
                self.stmt(body);
                self.scopes.pop();
            }
            TypedStmtKind::Return(v) => {
                if let Some(v) = v {
                    self.expr(v);
                }
            }
            TypedStmtKind::Break
            | TypedStmtKind::Continue
            | TypedStmtKind::Function(_)
            | TypedStmtKind::Needs(_)
            | TypedStmtKind::StructDecl { .. }
            | TypedStmtKind::EnumDecl { .. } => {}
        }
    }

    fn scoped(&mut self, s: &TypedStmt) {
        self.scopes.push(HashMap::new());
        self.stmt(s);
        self.scopes.pop();
    }

    fn expr(&mut self, e: &TypedExpr) {
        self.note_view_roots(e);
        match &e.kind {
            TypedExprKind::Int(_)
            | TypedExprKind::Float(_)
            | TypedExprKind::Bool(_)
            | TypedExprKind::Char(_)
            | TypedExprKind::String(_)
            | TypedExprKind::Null
            | TypedExprKind::Lambda(_)
            | TypedExprKind::LambdaInner { .. } => {}
            TypedExprKind::Identifier(n) => {
                if let Some(d) = self.resolve(n) {
                    self.ident_decl.insert(key(e), d);
                }
            }
            TypedExprKind::Assign { name, value } => match self.resolve(name) {
                Some(d) => {
                    let srcs = self.view_rhs(value);
                    self.edges.push((d, srcs));
                }
                None => self.expr(value),
            },
            TypedExprKind::Grouping(inner) | TypedExprKind::Deref(inner) => self.expr(inner),
            TypedExprKind::Unary { operand, .. } | TypedExprKind::Reference { operand, .. } => {
                self.expr(operand)
            }
            TypedExprKind::Cast { expr, .. } => self.expr(expr),
            TypedExprKind::Member { object, .. } => self.expr(object),
            TypedExprKind::Binary { left, right, .. }
            | TypedExprKind::And { left, right }
            | TypedExprKind::Or { left, right } => {
                self.expr(left);
                self.expr(right);
            }
            TypedExprKind::Index { object, index } => {
                self.expr(object);
                self.expr(index);
            }
            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
                self.expr(object);
                self.expr(index);
                self.expr(value);
            }
            TypedExprKind::FieldAssign { object, value, .. } => {
                self.expr(object);
                self.expr(value);
            }
            TypedExprKind::DerefAssign { target, value } => {
                self.expr(target);
                self.expr(value);
            }
            TypedExprKind::Slice { object, range } => {
                self.expr(object);
                self.expr(range);
            }
            TypedExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    self.expr(s);
                }
                if let Some(x) = end {
                    self.expr(x);
                }
            }
            TypedExprKind::FmtString(parts) => {
                for part in parts {
                    if let TypedFmtStringPart::Expr(x) = part {
                        self.expr(x);
                    }
                }
            }
            TypedExprKind::Call { callee, args } => {
                self.expr(callee);
                for a in args {
                    self.expr(a);
                }
            }
            TypedExprKind::EnumVariant { args, .. } => {
                for a in args {
                    self.expr(a);
                }
            }
            TypedExprKind::ArrayLiteral { elements }
            | TypedExprKind::VecLiteral { elements, .. } => {
                for x in elements {
                    self.expr(x);
                }
            }
            TypedExprKind::ArraySized { size, fill_value } => {
                self.expr(size);
                if let Some(v) = fill_value {
                    self.expr(v);
                }
            }
            TypedExprKind::StructLiteral { fields, .. } => {
                for (_, x) in fields {
                    self.expr(x);
                }
            }
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.expr(condition);
                self.expr(then_branch);
                self.expr(else_branch);
            }
            TypedExprKind::Match { scrutinee, arms } => {
                self.expr(scrutinee);
                for arm in arms {
                    self.scopes.push(HashMap::new());
                    if let TypedPattern::Variant { bindings, .. } = &arm.pattern {
                        for (name, _) in bindings {
                            self.scopes.last_mut().unwrap().insert(name.clone(), None);
                        }
                    }
                    self.expr(&arm.body);
                    self.scopes.pop();
                }
            }
            TypedExprKind::ResultAssert { scrutinee, .. } => self.expr(scrutinee),
            TypedExprKind::Block { stmts, tail } => self.block(stmts, Some(tail)),
        }
    }
}

#[derive(Clone)]
pub(crate) enum LeafStep {
    Field(String),
    Index(u64),
}

pub(crate) enum LeafPlan {
    Paths(Vec<Vec<(LeafStep, crate::AirType)>>),
    Whole(crate::AirType),
}

impl super::LoweringContext<'_> {
    fn view_is_text(&self, slot: crate::LocalId) -> bool {
        self.views.text_slots.contains(&slot)
    }

    fn view_slot_ty(&self, slot: crate::LocalId) -> crate::AirType {
        self.local_air_type(slot).unwrap_or_else(rc_slot_ty)
    }

    fn view_set(&mut self, slot: crate::LocalId, value: crate::Operand) {
        self.emit(
            crate::AirStmtKind::Assign {
                place: crate::Place::Local(slot),
                rvalue: crate::Rvalue::Use(value),
            },
            None,
        );
    }

    fn view_null_value(&self, slot: crate::LocalId) -> crate::Operand {
        if self.view_is_text(slot) {
            crate::Operand::Const(crate::AirConst::ZeroInit(self.view_slot_ty(slot)))
        } else {
            crate::Operand::Const(crate::AirConst::Null)
        }
    }

    fn view_null(&mut self, slot: crate::LocalId) {
        let null = self.view_null_value(slot);
        self.view_set(slot, null);
    }

    fn view_release_slot(&mut self, slot: crate::LocalId) {
        if self.view_is_text(slot) {
            self.emit_posed_release(slot, None);
        } else {
            self.emit_rc_release(slot, None);
        }
    }

    fn view_retain_slot(&mut self, slot: crate::LocalId) {
        if !self.view_is_text(slot) {
            self.emit_rc_retain(crate::Operand::Copy(slot), None);
            return;
        }
        let ty = self.view_slot_ty(slot);
        if matches!(ty, crate::AirType::Vec(_)) {
            self.emit_cow_retain(slot, None);
            return;
        }
        if self.counted_air(&ty) {
            self.emit_str_retain(slot, None);
        }
        if let Some(paths) = self.aggregate_rc_paths(&ty) {
            self.emit_carrier_field_retains(crate::Operand::Copy(slot), &ty, &paths, None);
        }
    }

    fn view_new_slots(&mut self, w: &Width, depth: usize) -> Vec<crate::LocalId> {
        let mut out = Vec::new();
        for ty in w.slot_types() {
            let text = ty != rc_slot_ty();
            let slot = self.alloc_temp_mut(ty);
            if text {
                self.views.text_slots.insert(slot);
            }
            self.views.entry_nulls.push(slot);
            self.views.slots.push((slot, depth));
            out.push(slot);
        }
        out
    }

    fn view_width(&mut self, decl: usize, line: u32) -> Width {
        let w = self
            .views
            .owners
            .width
            .get(&decl)
            .cloned()
            .unwrap_or_default();
        if w.rc() > MAX_WIDTH {
            self.report_unsupported(format!(
                "[rc-view] the view bound at line {line} may point into more than {MAX_WIDTH} \
                 `Rc` payloads"
            ));
            return Width::default();
        }
        if w.text() > MAX_WIDTH {
            self.report_unsupported(format!(
                "[rc-view] the view bound at line {line} may point into more than {MAX_WIDTH} \
                 strings"
            ));
            return Width::default();
        }
        w
    }

    fn view_leaf_plan(&self, view_ty: &InferType) -> Result<Option<LeafPlan>, &'static str> {
        if let InferType::Ref { referent, .. } = view_ty {
            let target = self.lower_type_from_infer(referent);
            if let crate::AirType::Vec(elem) = &target {
                if self.counted_air(elem) {
                    return Ok(Some(LeafPlan::Whole(target)));
                }
                return Ok(None);
            }
            let mut out = Vec::new();
            let ok = self.view_str_leaves(&target, &mut Vec::new(), &mut out);
            if ok && out.len() <= MAX_WIDTH {
                return Ok((!out.is_empty()).then_some(LeafPlan::Paths(out)));
            }
            return Ok(Some(LeafPlan::Whole(target)));
        }
        if !matches!(view_ty, InferType::Slice { .. }) {
            return Ok(None);
        }
        let crate::AirType::Slice(elem) = self.lower_type_from_infer(view_ty) else {
            return Ok(None);
        };
        let mut probe = Vec::new();
        let ok = self.view_str_leaves(&elem, &mut Vec::new(), &mut probe);
        if ok && probe.is_empty() {
            return Ok(None);
        }
        Err("a slice of strings")
    }

    fn view_leaf_width(&self, view_ty: &InferType) -> Result<Width, &'static str> {
        Ok(match self.view_leaf_plan(view_ty)? {
            None => Width::default(),
            Some(LeafPlan::Paths(p)) => Width::of(crate::AirType::Str, p.len()),
            Some(LeafPlan::Whole(ty)) => Width::of(ty, 1),
        })
    }

    fn view_str_leaves(
        &self,
        ty: &crate::AirType,
        prefix: &mut Vec<(LeafStep, crate::AirType)>,
        out: &mut Vec<Vec<(LeafStep, crate::AirType)>>,
    ) -> bool {
        match ty {
            crate::AirType::Str => {
                out.push(prefix.clone());
                true
            }
            crate::AirType::Struct(name) => {
                let Some(def) = self.defs().find_struct(name) else {
                    return !self.counted_air(ty);
                };
                let fields: Vec<(String, crate::AirType)> = def
                    .fields
                    .iter()
                    .map(|f| (f.name.clone(), f.ty.clone()))
                    .collect();
                for (fname, fty) in fields {
                    prefix.push((LeafStep::Field(fname), fty.clone()));
                    let ok = self.view_str_leaves(&fty, prefix, out);
                    prefix.pop();
                    if !ok {
                        return false;
                    }
                }
                true
            }
            crate::AirType::Array(inner, n) => {
                for i in 0..*n {
                    prefix.push((LeafStep::Index(i), (**inner).clone()));
                    let ok = self.view_str_leaves(inner, prefix, out);
                    prefix.pop();
                    if !ok {
                        return false;
                    }
                }
                true
            }
            crate::AirType::Ptr(_)
            | crate::AirType::I8
            | crate::AirType::I16
            | crate::AirType::I32
            | crate::AirType::I64
            | crate::AirType::U8
            | crate::AirType::U16
            | crate::AirType::U32
            | crate::AirType::U64
            | crate::AirType::F32
            | crate::AirType::F64
            | crate::AirType::Bool
            | crate::AirType::Char
            | crate::AirType::Slice(_)
            | crate::AirType::FnPtr { .. }
            | crate::AirType::Void => true,
            crate::AirType::Enum(_)
            | crate::AirType::Vec(_)
            | crate::AirType::Param(_)
            | crate::AirType::Opaque => !self.counted_air(ty),
        }
    }

    pub(super) fn view_prepare(&mut self, params: &[TypedParam], body: &[TypedStmt]) {
        let leaves = |ty: &InferType| self.view_leaf_width(ty);
        let owners = analyze(params, body, &leaves);
        self.views.owners = owners;
        let mut refused: Vec<(u32, &'static str)> = self
            .views
            .owners
            .leaf_nodes
            .values()
            .filter_map(|(plan, line)| plan.as_ref().err().map(|what| (*line, *what)))
            .collect();
        refused.sort_unstable();
        refused.dedup();
        for (line, what) in refused {
            self.report_unsupported(format!(
                "[rc-view] a byte view returned from {what} this function does not own is not \
                 supported yet (line {line})"
            ));
        }
        for param in params {
            let raw = self
                .views
                .owners
                .width
                .get(&key(param))
                .cloned()
                .unwrap_or_default();
            let w = self.view_width(key(param), param.span.line);
            let Some(local) = self.lookup_local(&param.name) else {
                continue;
            };
            if raw.over() {
                self.view_mark_overflow(local);
            }
            if !w.is_empty() {
                let slots = self.view_new_slots(&w, 0);
                self.views.slot_of.insert(local, slots);
            }
        }
    }

    pub(super) fn view_let_width(&mut self, stmt: &TypedStmt) -> Option<Width> {
        let w = self
            .views
            .owners
            .width
            .get(&key(stmt))
            .cloned()
            .unwrap_or_default();
        if w.over() {
            self.view_width(key(stmt), stmt.span.line);
            return None;
        }
        Some(w)
    }

    pub(super) fn view_slots_for_let(&mut self, w: Width) -> Option<Vec<crate::LocalId>> {
        (!w.is_empty()).then(|| {
            let depth = self.locals_by_name.len();
            self.view_new_slots(&w, depth)
        })
    }

    pub(super) fn view_mark_overflow(&mut self, local: crate::LocalId) {
        self.views.overflow.insert(local);
    }

    pub(super) fn view_assign_begin(
        &mut self,
        name: &str,
        rhs: &TypedExpr,
    ) -> Option<Vec<crate::LocalId>> {
        let target = self.lookup_local(name);
        if target.is_some_and(|id| self.views.overflow.contains(&id)) {
            return None;
        }
        let has_sources = !sources(rhs, &self.views.owners).is_empty();
        if target.is_none() && has_sources {
            self.report_unsupported(format!(
                "[rc-view] the global `{name}` cannot keep alive the `Rc` its view points into"
            ));
            return None;
        }
        if target.is_some_and(|id| self.capture_slots.contains_key(&id)) && has_sources {
            self.report_unsupported(format!(
                "[rc-view] `{name}` is captured, and a view stored into it from a closure \
                 cannot keep its `Rc` alive"
            ));
            return None;
        }
        Some(self.view_rhs_begin(rhs))
    }

    pub(super) fn view_bind(&mut self, local: crate::LocalId, slots: Vec<crate::LocalId>) {
        self.views.slot_of.insert(local, slots);
    }

    pub(super) fn view_slots_of(&self, local: crate::LocalId) -> Option<Vec<crate::LocalId>> {
        self.views.slot_of.get(&local).cloned()
    }

    pub(super) fn view_rhs_begin(&mut self, rhs: &TypedExpr) -> Vec<crate::LocalId> {
        let srcs = sources(rhs, &self.views.owners);
        let depth = self.locals_by_name.len();
        let mut all = Vec::new();
        for (node, src) in srcs {
            let tmps = self.view_new_slots(&src.width(), depth);
            all.extend(tmps.iter().copied());
            self.views.captures.push((node, tmps, false, src.kind()));
        }
        all
    }

    fn view_captures_end(&mut self, n_tmps: usize) {
        let mut seen = 0;
        while seen < n_tmps {
            let Some((_, tmps, hit, _)) = self.views.captures.pop() else {
                break;
            };
            seen += tmps.len();
            if self.views.pending_leaves.iter().any(|p| p.1 == tmps) {
                self.views.pending_leaves.retain(|p| p.1 != tmps);
                if !self.last_block_is_terminated() {
                    self.report_ice(
                        "ICE: a call that returns a view was never lowered after its argument"
                            .to_string(),
                    );
                }
            }
            if !hit && !self.last_block_is_terminated() {
                self.report_ice(
                    "ICE: the owner of a view was never lowered, so no share of its `Rc` was taken"
                        .to_string(),
                );
            }
        }
    }

    pub(super) fn view_rhs_end(
        &mut self,
        tmps: Vec<crate::LocalId>,
        slots: Option<Vec<crate::LocalId>>,
        replaces: bool,
    ) {
        self.view_captures_end(tmps.len());
        if self.last_block_is_terminated() {
            return;
        }
        let Some(slots) = slots else {
            if !tmps.is_empty() {
                self.report_ice(
                    "ICE: a view binding receives an `Rc` owner but has no owner slot".to_string(),
                );
            }
            return;
        };
        let mut kinds: Vec<crate::AirType> = Vec::new();
        for slot in slots.iter().chain(tmps.iter()) {
            let ty = self.view_slot_ty(*slot);
            if !kinds.contains(&ty) {
                kinds.push(ty);
            }
        }
        for kind in kinds {
            let group_tmps: Vec<crate::LocalId> = tmps
                .iter()
                .copied()
                .filter(|t| self.view_slot_ty(*t) == kind)
                .collect();
            let group_slots: Vec<crate::LocalId> = slots
                .iter()
                .copied()
                .filter(|t| self.view_slot_ty(*t) == kind)
                .collect();
            if group_tmps.len() > group_slots.len() {
                self.report_ice(format!(
                    "ICE: a view needs {} owner slots and its binding has {}",
                    group_tmps.len(),
                    group_slots.len()
                ));
                return;
            }
            for (j, slot) in group_slots.iter().enumerate() {
                if replaces {
                    self.view_release_slot(*slot);
                }
                match group_tmps.get(j) {
                    Some(tmp) => {
                        self.view_set(*slot, crate::Operand::Copy(*tmp));
                        self.view_null(*tmp);
                    }
                    None => self.view_null(*slot),
                }
            }
        }
    }

    pub(super) fn view_hold_release(&mut self, tmps: &[crate::LocalId]) {
        if !self.last_block_is_terminated() {
            for tmp in tmps {
                self.view_release_slot(*tmp);
                self.view_null(*tmp);
            }
        }
        self.views.slots.retain(|(s, _)| !tmps.contains(s));
    }

    // the callee may replace the string before it returns a view into the new one
    fn view_load_pending_leaves(&mut self, call: usize) {
        let (due, keep): (Vec<_>, Vec<_>) = std::mem::take(&mut self.views.pending_leaves)
            .into_iter()
            .partition(|p| p.0 == call);
        self.views.pending_leaves = keep;
        if self.last_block_is_terminated() {
            return;
        }
        for (_, tmps, base, plan) in due {
            match plan {
                LeafPlan::Paths(paths) => {
                    for (tmp, path) in tmps.iter().zip(paths.iter()) {
                        self.view_release_slot(*tmp);
                        self.load_leaf(*tmp, base, path);
                        self.view_retain_slot(*tmp);
                    }
                }
                LeafPlan::Whole(_) => {
                    for tmp in &tmps {
                        self.view_release_slot(*tmp);
                        self.load_leaf(*tmp, base, &[]);
                        self.view_retain_slot(*tmp);
                    }
                }
            }
        }
    }

    pub(super) fn view_capture(&mut self, e: &TypedExpr, value: Option<&crate::Operand>) {
        let k = key(e);
        if value.is_some() && self.views.pending_leaves.iter().any(|p| p.0 == k) {
            self.view_load_pending_leaves(k);
        }
        let positions: Vec<usize> = (0..self.views.captures.len())
            .filter(|i| self.views.captures[*i].0 == k)
            .collect();
        for pos in positions {
            let kind = self.views.captures[pos].3;
            let tmps = self.views.captures[pos].1.clone();
            let values: Vec<crate::Operand> = match (kind, value) {
                (CapKind::Rc | CapKind::Text, Some(v)) => vec![v.clone()],
                (CapKind::Rc, None) => {
                    let TypedExprKind::Identifier(name) = &e.kind else {
                        continue;
                    };
                    let Some(id) = self.lookup_local(name) else {
                        continue;
                    };
                    if self.capture_slots.contains_key(&id) {
                        continue;
                    }
                    vec![crate::Operand::Copy(id)]
                }
                (CapKind::Text | CapKind::Leaves(_), None) => continue,
                (CapKind::Leaves(call), Some(v)) => {
                    self.views.captures[pos].2 = true;
                    if self.last_block_is_terminated() {
                        return;
                    }
                    let ptr_ty = self.lower_type_from_infer(&e.ty);
                    let base = self.operand_to_local(v.clone(), &ptr_ty);
                    let plan = self.view_leaf_plan(&e.ty).ok().flatten();
                    if let Some(plan) = plan {
                        self.views.pending_leaves.retain(|p| p.1 != tmps);
                        self.views.pending_leaves.push((call, tmps, base, plan));
                    }
                    continue;
                }
                (CapKind::Ident, _) => {
                    let TypedExprKind::Identifier(name) = &e.kind else {
                        continue;
                    };
                    let Some(slots) = self
                        .lookup_local(name)
                        .and_then(|l| self.views.slot_of.get(&l).cloned())
                    else {
                        self.report_ice(format!("ICE: view binding `{name}` has no owner slot"));
                        return;
                    };
                    slots.into_iter().map(crate::Operand::Copy).collect()
                }
            };
            self.views.captures[pos].2 = true;
            if self.last_block_is_terminated() {
                return;
            }
            for (tmp, v) in tmps.iter().zip(values.iter()) {
                self.view_release_slot(*tmp);
                self.view_set(*tmp, v.clone());
                self.view_retain_slot(*tmp);
            }
        }
    }

    pub(super) fn view_hold(&mut self, e: &TypedExpr) -> Option<Vec<crate::LocalId>> {
        if !is_view(&e.ty) || sources(e, &self.views.owners).is_empty() {
            return None;
        }
        Some(self.view_rhs_begin(e))
    }

    pub(super) fn view_hold_done(&mut self, tmps: Option<Vec<crate::LocalId>>) {
        if let Some(tmps) = tmps {
            self.view_captures_end(tmps.len());
            self.views.held.extend(tmps);
        }
    }

    pub(super) fn view_release_held(&mut self, mark: usize) {
        let tmps: Vec<crate::LocalId> = self.views.held.drain(mark..).collect();
        if !tmps.is_empty() {
            self.view_hold_release(&tmps);
        }
    }

    pub(super) fn view_release_below(&mut self, depth: usize, forget: bool) {
        let below: Vec<crate::LocalId> = self
            .views
            .slots
            .iter()
            .filter(|(_, d)| *d > depth)
            .map(|(s, _)| *s)
            .collect();
        if !self.last_block_is_terminated() {
            for slot in below {
                self.view_release_slot(slot);
                self.view_null(slot);
            }
        }
        if forget {
            self.views.slots.retain(|(_, d)| *d <= depth);
        }
    }

    // the codegen leaves locals uninitialised and a slot is released before every write
    pub(super) fn view_hoist_nulls(&mut self) {
        let nulls = std::mem::take(&mut self.views.entry_nulls);
        let stmts: Vec<crate::AirStmt> = nulls
            .into_iter()
            .map(|slot| crate::AirStmt {
                kind: crate::AirStmtKind::Assign {
                    place: crate::Place::Local(slot),
                    rvalue: crate::Rvalue::Use(self.view_null_value(slot)),
                },
                span: None,
            })
            .collect();
        let Some(entry) = self.current_blocks.first_mut() else {
            return;
        };
        entry.stmts.splice(0..0, stmts);
    }

    pub(super) fn view_release_in_flight(&mut self, (captures, held): (usize, usize)) {
        if self.last_block_is_terminated() {
            return;
        }
        let mut tmps: Vec<crate::LocalId> = self.views.captures
            [captures.min(self.views.captures.len())..]
            .iter()
            .flat_map(|c| c.1.iter().copied())
            .collect();
        tmps.extend(
            self.views.held[held.min(self.views.held.len())..]
                .iter()
                .copied(),
        );
        for tmp in tmps {
            self.view_release_slot(tmp);
            self.view_null(tmp);
        }
    }

    pub(super) fn view_release_params_on_fallthrough(&mut self) {
        if !self.last_block_is_terminated() {
            self.view_release_all_for_return();
        }
        self.views.slots.clear();
    }

    pub(super) fn view_release_all_for_return(&mut self) {
        let all: Vec<crate::LocalId> = self.views.slots.iter().map(|(s, _)| *s).collect();
        for slot in all {
            self.view_release_slot(slot);
        }
    }
}

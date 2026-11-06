use aelys_sema::{
    InferType, TypedExpr, TypedExprKind, TypedFmtStringPart, TypedParam, TypedPattern, TypedStmt,
    TypedStmtKind,
};
use std::collections::HashMap;

pub(super) fn is_view(ty: &InferType) -> bool {
    matches!(ty, InferType::Slice { .. } | InferType::Ref { .. })
}

pub(super) fn key<T>(x: &T) -> usize {
    x as *const T as usize
}

pub(super) const MAX_WIDTH: usize = 8;

#[derive(Default)]
pub(crate) struct ViewOwners {
    pub width: HashMap<usize, usize>,
    pub ident_width: HashMap<usize, usize>,
}

#[derive(Default)]
struct Pass {
    scopes: Vec<HashMap<String, Option<usize>>>,
    edges: Vec<(usize, Vec<SrcRef>)>,
    ident_decl: HashMap<usize, usize>,
}

#[derive(Clone, Copy)]
enum SrcRef {
    Rc,
    Decl(usize),
}

pub(super) fn sources(expr: &TypedExpr, owners: &ViewOwners) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    collect(expr, &mut |e| {
        if matches!(e.ty, InferType::Rc(_)) {
            out.push((key(e), 1));
        } else if let Some(w) = owners.ident_width.get(&key(e))
            && *w <= MAX_WIDTH
        {
            out.push((key(e), *w));
        }
    });
    out
}

fn collect<'e>(expr: &'e TypedExpr, f: &mut dyn FnMut(&'e TypedExpr)) {
    match &expr.kind {
        TypedExprKind::Grouping(inner) | TypedExprKind::Deref(inner) => collect(inner, f),
        TypedExprKind::Slice { object, .. }
        | TypedExprKind::Index { object, .. }
        | TypedExprKind::Member { object, .. } => collect(object, f),
        TypedExprKind::Reference { operand, .. } => collect(operand, f),
        TypedExprKind::EnumVariant {
            enum_name,
            variant,
            args,
            ..
        } if enum_name == "Rc" && variant == "get" => {
            if let Some(h) = args.first() {
                f(h);
            }
        }
        TypedExprKind::Identifier(_) if !matches!(expr.ty, InferType::Rc(_)) => f(expr),
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
                    f(a);
                } else if is_view(&a.ty) {
                    collect(a, f);
                }
            }
        }
        _ => {}
    }
}

pub(super) fn analyze(params: &[TypedParam], body: &[TypedStmt]) -> ViewOwners {
    let mut p = Pass::default();
    p.scopes.push(HashMap::new());
    for param in params {
        let decl = is_view(&param.ty).then(|| key(param));
        p.scopes[0].insert(param.name.clone(), decl);
    }
    for s in body {
        p.stmt(s);
    }
    let mut width: HashMap<usize, usize> = HashMap::new();
    loop {
        let mut changed = false;
        for (decl, srcs) in &p.edges {
            let need: usize = srcs
                .iter()
                .map(|s| match s {
                    SrcRef::Rc => 1,
                    SrcRef::Decl(d) => width.get(d).copied().unwrap_or(0),
                })
                .sum::<usize>()
                .min(MAX_WIDTH + 1);
            let have = width.get(decl).copied().unwrap_or(0);
            if need > have {
                width.insert(*decl, need);
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
        .filter_map(|(i, d)| width.get(d).map(|w| (*i, *w)))
        .collect();
    ViewOwners { width, ident_width }
}

impl Pass {
    fn resolve(&self, name: &str) -> Option<usize> {
        for scope in self.scopes.iter().rev() {
            if let Some(d) = scope.get(name) {
                return *d;
            }
        }
        None
    }

    fn srcs_of(&mut self, rhs: &TypedExpr) -> Vec<SrcRef> {
        let mut found: Vec<&TypedExpr> = Vec::new();
        collect(rhs, &mut |e| found.push(e));
        let mut out = Vec::new();
        for e in found {
            if matches!(e.ty, InferType::Rc(_)) {
                out.push(SrcRef::Rc);
            } else if let Some(d) = self.ident_decl.get(&key(e)) {
                out.push(SrcRef::Decl(*d));
            }
        }
        out
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

impl super::LoweringContext<'_> {
    fn view_ptr_ty() -> crate::AirType {
        crate::AirType::Ptr(Box::new(crate::AirType::Void))
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

    fn view_null(&mut self, slot: crate::LocalId) {
        self.view_set(slot, crate::Operand::Const(crate::AirConst::Null));
    }

    fn view_new_slots(&mut self, n: usize, depth: usize) -> Vec<crate::LocalId> {
        (0..n)
            .map(|_| {
                let slot = self.alloc_temp_mut(Self::view_ptr_ty());
                self.views.entry_nulls.push(slot);
                self.views.slots.push((slot, depth));
                slot
            })
            .collect()
    }

    fn view_width(&mut self, decl: usize, line: u32) -> usize {
        let w = self.views.owners.width.get(&decl).copied().unwrap_or(0);
        if w > MAX_WIDTH {
            self.report_unsupported(format!(
                "[rc-view] the view bound at line {line} may point into more than {MAX_WIDTH} \
                 `Rc` payloads"
            ));
            return 0;
        }
        w
    }

    pub(super) fn view_prepare(&mut self, params: &[TypedParam], body: &[TypedStmt]) {
        self.views.owners = analyze(params, body);
        for param in params {
            let raw = self
                .views
                .owners
                .width
                .get(&key(param))
                .copied()
                .unwrap_or(0);
            let w = self.view_width(key(param), param.span.line);
            let Some(local) = self.lookup_local(&param.name) else {
                continue;
            };
            if raw > MAX_WIDTH {
                self.view_mark_overflow(local);
            }
            if w > 0 {
                let slots = self.view_new_slots(w, 0);
                self.views.slot_of.insert(local, slots);
            }
        }
    }

    pub(super) fn view_let_width(&mut self, stmt: &TypedStmt) -> Option<usize> {
        let w = self
            .views
            .owners
            .width
            .get(&key(stmt))
            .copied()
            .unwrap_or(0);
        if w > MAX_WIDTH {
            self.view_width(key(stmt), stmt.span.line);
            return None;
        }
        Some(w)
    }

    pub(super) fn view_slots_for_let(&mut self, w: usize) -> Option<Vec<crate::LocalId>> {
        (w > 0).then(|| {
            let depth = self.locals_by_name.len();
            self.view_new_slots(w, depth)
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
        for (node, w) in srcs {
            let tmps = self.view_new_slots(w, depth);
            all.extend(tmps.iter().copied());
            self.views.captures.push((node, tmps, false));
        }
        all
    }

    fn view_captures_end(&mut self, n_tmps: usize) {
        let mut seen = 0;
        while seen < n_tmps {
            let Some((_, tmps, hit)) = self.views.captures.pop() else {
                break;
            };
            seen += tmps.len();
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
        if tmps.len() > slots.len() {
            self.report_ice(format!(
                "ICE: a view needs {} owner slots and its binding has {}",
                tmps.len(),
                slots.len()
            ));
            return;
        }
        for (j, slot) in slots.iter().enumerate() {
            if replaces {
                self.emit_rc_release(*slot, None);
            }
            match tmps.get(j) {
                Some(tmp) => {
                    self.view_set(*slot, crate::Operand::Copy(*tmp));
                    self.view_null(*tmp);
                }
                None => self.view_null(*slot),
            }
        }
    }

    pub(super) fn view_hold_release(&mut self, tmps: &[crate::LocalId]) {
        if !self.last_block_is_terminated() {
            for tmp in tmps {
                self.emit_rc_release(*tmp, None);
                self.view_null(*tmp);
            }
        }
        self.views.slots.retain(|(s, _)| !tmps.contains(s));
    }

    pub(super) fn view_capture(&mut self, e: &TypedExpr, value: Option<&crate::Operand>) {
        let k = key(e);
        let positions: Vec<usize> = (0..self.views.captures.len())
            .filter(|i| self.views.captures[*i].0 == k)
            .collect();
        if positions.is_empty() {
            return;
        }
        let is_rc = matches!(e.ty, InferType::Rc(_));
        let values: Vec<crate::Operand> = if let (true, Some(v)) = (is_rc, value) {
            vec![v.clone()]
        } else {
            let TypedExprKind::Identifier(name) = &e.kind else {
                return;
            };
            let Some(slots) = self
                .lookup_local(name)
                .and_then(|l| self.views.slot_of.get(&l).cloned())
            else {
                self.report_ice(format!("ICE: view binding `{name}` has no owner slot"));
                return;
            };
            slots.into_iter().map(crate::Operand::Copy).collect()
        };
        for pos in positions {
            self.views.captures[pos].2 = true;
            let tmps = self.views.captures[pos].1.clone();
            if self.last_block_is_terminated() {
                return;
            }
            for (tmp, v) in tmps.iter().zip(values.iter()) {
                self.emit_rc_release(*tmp, None);
                self.view_set(*tmp, v.clone());
                self.emit_rc_retain(crate::Operand::Copy(*tmp), None);
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
                self.emit_rc_release(slot, None);
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
        let Some(entry) = self.current_blocks.first_mut() else {
            return;
        };
        let stmts: Vec<crate::AirStmt> = nulls
            .into_iter()
            .map(|slot| crate::AirStmt {
                kind: crate::AirStmtKind::Assign {
                    place: crate::Place::Local(slot),
                    rvalue: crate::Rvalue::Use(crate::Operand::Const(crate::AirConst::Null)),
                },
                span: None,
            })
            .collect();
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
            self.emit_rc_release(tmp, None);
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
            self.emit_rc_release(slot, None);
        }
    }
}

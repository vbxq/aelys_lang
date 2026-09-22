use super::LoweringContext;
use crate::*;
use aelys_sema::{
    InferType, TypedExpr, TypedExprKind, TypedFmtStringPart, TypedStmt, TypedStmtKind,
};

// only a form known to free nothing answers false, so a new form fails closed
pub(super) fn may_free(expr: &TypedExpr) -> bool {
    match &expr.kind {
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Char(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Null
        | TypedExprKind::Identifier(_)
        | TypedExprKind::Lambda(_)
        | TypedExprKind::LambdaInner { .. } => false,
        TypedExprKind::Grouping(inner) | TypedExprKind::Deref(inner) => may_free(inner),
        TypedExprKind::Unary { operand, .. } | TypedExprKind::Reference { operand, .. } => {
            may_free(operand)
        }
        TypedExprKind::Cast { expr, .. } => may_free(expr),
        TypedExprKind::Member { object, .. } => may_free(object),
        TypedExprKind::Binary { left, right, .. }
        | TypedExprKind::And { left, right }
        | TypedExprKind::Or { left, right } => may_free(left) || may_free(right),
        TypedExprKind::Index { object, index } => may_free(object) || may_free(index),
        TypedExprKind::FmtString(parts) => parts
            .iter()
            .any(|part| matches!(part, TypedFmtStringPart::Expr(e) if may_free(e))),
        TypedExprKind::ArrayLiteral { elements } | TypedExprKind::VecLiteral { elements, .. } => {
            elements.iter().any(may_free)
        }
        TypedExprKind::StructLiteral { fields, .. } => fields.iter().any(|(_, e)| may_free(e)),
        TypedExprKind::EnumVariant { args, .. } => args.iter().any(may_free),
        _ => true,
    }
}

fn names(expr: &TypedExpr, name: &str) -> bool {
    matches!(&peel(expr).kind, TypedExprKind::Identifier(n) if n == name)
}

pub(super) fn may_write_local(expr: &TypedExpr, name: &str) -> bool {
    let w = |e: &TypedExpr| may_write_local(e, name);
    match &expr.kind {
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::Char(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Null
        | TypedExprKind::Identifier(_)
        | TypedExprKind::Lambda(_)
        | TypedExprKind::LambdaInner { .. } => false,
        TypedExprKind::Assign { name: n, value } => n == name || w(value),
        TypedExprKind::Reference { mutable, operand } => {
            (*mutable && names(operand, name)) || w(operand)
        }
        TypedExprKind::IndexAssign {
            object,
            index,
            value,
        } => names(object, name) || w(object) || w(index) || w(value),
        TypedExprKind::FieldAssign { object, value, .. } => {
            names(object, name) || w(object) || w(value)
        }
        TypedExprKind::DerefAssign { target, value } => w(target) || w(value),
        TypedExprKind::Grouping(inner) | TypedExprKind::Deref(inner) => w(inner),
        TypedExprKind::Unary { operand, .. } => w(operand),
        TypedExprKind::Cast { expr, .. } => w(expr),
        TypedExprKind::Member { object, .. } => w(object),
        TypedExprKind::Binary { left, right, .. }
        | TypedExprKind::And { left, right }
        | TypedExprKind::Or { left, right } => w(left) || w(right),
        TypedExprKind::Index { object, index } => w(object) || w(index),
        TypedExprKind::Slice { object, range } => w(object) || w(range),
        TypedExprKind::Range { start, end, .. } => {
            start.as_deref().is_some_and(w) || end.as_deref().is_some_and(w)
        }
        TypedExprKind::FmtString(parts) => parts
            .iter()
            .any(|part| matches!(part, TypedFmtStringPart::Expr(e) if w(e))),
        TypedExprKind::Call { callee, args } => w(callee) || args.iter().any(w),
        TypedExprKind::EnumVariant {
            enum_name, args, ..
        } => args
            .iter()
            .any(|a| (enum_name == "Vec" && names(a, name)) || w(a)),
        TypedExprKind::ArrayLiteral { elements } | TypedExprKind::VecLiteral { elements, .. } => {
            elements.iter().any(w)
        }
        TypedExprKind::ArraySized { size, fill_value } => {
            w(size) || fill_value.as_deref().is_some_and(w)
        }
        TypedExprKind::StructLiteral { fields, .. } => fields.iter().any(|(_, e)| w(e)),
        TypedExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => w(condition) || w(then_branch) || w(else_branch),
        TypedExprKind::Match { scrutinee, arms } => {
            w(scrutinee) || arms.iter().any(|arm| w(&arm.body))
        }
        TypedExprKind::ResultAssert { scrutinee, .. } => w(scrutinee),
        TypedExprKind::Block { stmts, tail } => {
            stmts.iter().any(|s| stmt_may_write_local(s, name)) || w(tail)
        }
    }
}

fn stmt_may_write_local(stmt: &TypedStmt, name: &str) -> bool {
    let w = |e: &TypedExpr| may_write_local(e, name);
    let ws = |s: &TypedStmt| stmt_may_write_local(s, name);
    match &stmt.kind {
        TypedStmtKind::Expression(e) => w(e),
        TypedStmtKind::Let { initializer, .. } => w(initializer),
        TypedStmtKind::Block(stmts) => stmts.iter().any(ws),
        TypedStmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => w(condition) || ws(then_branch) || else_branch.as_deref().is_some_and(ws),
        TypedStmtKind::While { condition, body } => w(condition) || ws(body),
        TypedStmtKind::For {
            start,
            end,
            step,
            body,
            ..
        } => w(start) || w(end) || step.as_ref().as_ref().is_some_and(w) || ws(body),
        TypedStmtKind::ForEach { iterable, body, .. } => w(iterable) || ws(body),
        TypedStmtKind::Return(value) => value.as_ref().is_some_and(w),
        TypedStmtKind::Break
        | TypedStmtKind::Continue
        | TypedStmtKind::Function(_)
        | TypedStmtKind::Needs(_)
        | TypedStmtKind::StructDecl { .. }
        | TypedStmtKind::EnumDecl { .. } => false,
    }
}

pub(super) fn peel(expr: &TypedExpr) -> &TypedExpr {
    let mut e = expr;
    while let TypedExprKind::Grouping(inner) = &e.kind {
        e = inner;
    }
    e
}

impl<'a> LoweringContext<'a> {
    // a read through a vec element, a slice or a reference borrows a share a sibling may drop
    pub(super) fn reads_a_borrowed_place(&self, expr: &TypedExpr) -> bool {
        let e = peel(expr);
        if !self.counted(&e.ty) {
            return false;
        }
        match &e.kind {
            TypedExprKind::Index { object, .. } => {
                Self::roots_a_vec(&object.ty)
                    || Self::views_a_slice(&object.ty)
                    || Self::roots_an_array(&object.ty)
            }
            TypedExprKind::Deref(_) | TypedExprKind::Member { .. } => true,
            // a global hands out what it holds, and a sibling that runs code may replace it
            TypedExprKind::Identifier(name) => self.is_global_name(name),
            TypedExprKind::EnumVariant {
                enum_name, variant, ..
            } => enum_name == "Rc" && variant == "get",
            TypedExprKind::Call { callee, .. } => {
                matches!(self.named_callee(callee), Some(Callee::Named(name))
                    if !self.retaining_fns.contains(&name)
                        && !crate::symbols::BOOTSTRAP_BUILTIN_SYMBOLS.contains(&name.as_str()))
            }
            _ => false,
        }
    }

    pub(super) fn pin_str_read(&mut self, val: Operand, sp: Option<Span>) -> Operand {
        let pinned = self.retained_str_copy(val, sp);
        if let Operand::Copy(id) = pinned
            && !self.last_block_is_terminated()
        {
            self.stmt_str_temps.push((id, self.current_blocks.len()));
        }
        pinned
    }

    // one entry for every sibling operand: a borrowed read or a rewritten local is held as read
    pub(super) fn hold_operand(
        &mut self,
        op: Operand,
        e: &TypedExpr,
        later: &[&TypedExpr],
        lends_mut: bool,
        sp: Option<Span>,
    ) -> Operand {
        let frees = lends_mut || later.iter().any(|s| may_free(s));
        let borrowed =
            self.reads_a_borrowed_place(e) || matches!(peel(e).kind, TypedExprKind::Deref(_));
        if frees && borrowed {
            return self.snapshot_value(op, e, sp);
        }
        let TypedExprKind::Identifier(name) = &peel(e).kind else {
            return op;
        };
        if !later.iter().any(|s| may_write_local(s, name)) {
            return op;
        }
        self.snapshot_value(op, e, sp)
    }

    // the old value dies only where the store roots in a binding this frame owns
    pub(super) fn store_root_owns(&self, expr: &TypedExpr) -> bool {
        let e = peel(expr);
        match &e.kind {
            TypedExprKind::Identifier(name) => {
                self.lookup_local(name).is_some() && !matches!(e.ty, InferType::Rc(_))
            }
            TypedExprKind::Member { object, .. } | TypedExprKind::Index { object, .. } => {
                !matches!(object.ty, InferType::Rc(_))
                    && !Self::views_a_slice(&object.ty)
                    && self.store_root_owns(object)
            }
            TypedExprKind::Deref(inner) => matches!(inner.ty, InferType::Ref { mutable: true, .. }),
            _ => false,
        }
    }

    fn snapshot_value(&mut self, op: Operand, e: &TypedExpr, sp: Option<Span>) -> Operand {
        if self.counted(&e.ty) {
            return self.pin_str_read(op, sp);
        }
        let ty = self.lower_type_from_infer(&e.ty);
        let plain = match &e.ty {
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
            | InferType::Char
            | InferType::Ref { .. }
            | InferType::Slice { .. }
            | InferType::Function { .. } => true,
            InferType::Array(inner, _) if self.counted(inner) => true,
            InferType::Array(..) | InferType::Struct(_) | InferType::Enum(..) => {
                !crate::rc_paths::air_type_has_rc(&ty, &self.structs, &self.enums)
            }
            // a value mono may still turn into a string is never copied without its share
            InferType::String
            | InferType::Vec(_)
            | InferType::Rc(_)
            | InferType::Null
            | InferType::Never
            | InferType::Tuple(_)
            | InferType::Range
            | InferType::Var(_)
            | InferType::Dynamic => false,
        };
        if plain {
            return self.emit_rvalue_to_temp(ty, Rvalue::Use(op), sp);
        }
        if !matches!(e.ty, InferType::Vec(_)) {
            return op;
        }
        let copy = self.alloc_temp(ty);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(copy),
                rvalue: Rvalue::Use(op),
            },
            sp,
        );
        self.emit_cow_retain(copy, sp);
        if !self.last_block_is_terminated() {
            self.stmt_str_temps.push((copy, self.current_blocks.len()));
        }
        Operand::Copy(copy)
    }

    pub(super) fn lower_pinned_operands(
        &mut self,
        exprs: &[&TypedExpr],
        sp: Option<Span>,
    ) -> Vec<Operand> {
        self.lower_pinned_operands_owning(exprs, false, sp)
    }

    // the share is taken as each operand is read, so a later sibling cannot change what it holds
    pub(super) fn lower_pinned_operands_owning(
        &mut self,
        exprs: &[&TypedExpr],
        own_each: bool,
        sp: Option<Span>,
    ) -> Vec<Operand> {
        let consumer_takes_mut = exprs
            .iter()
            .any(|e| matches!(e.ty, InferType::Ref { mutable: true, .. }));
        exprs
            .iter()
            .enumerate()
            .map(|(i, e)| {
                let op = self.lower_expr(e);
                let op = self.hold_operand(op, e, &exprs[i + 1..], consumer_takes_mut, sp);
                if own_each {
                    self.own_str_for_store(op, &e.ty, sp)
                } else {
                    op
                }
            })
            .collect()
    }
}

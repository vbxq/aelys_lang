// every move- or control-flow-bearing form so no move is silently dropped from the analysis.

use std::collections::HashSet;

use aelys_sema::{
    InferType, TypeTable, TypedExpr, TypedExprKind, TypedFmtStringPart, TypedFunction,
    TypedMatchArm, TypedProgram, TypedStmt, TypedStmtKind,
};
use aelys_syntax::Span;

use super::category::{Category, category};
use super::*;

pub fn build_program(program: &TypedProgram) -> BirProgram {
    let tt = &program.type_table;
    // distinguishable from an indirect call at every call site (mirrors lower_callee)
    let mut fn_names = gather_fn_names(&program.stmts);
    let globals = gather_global_names(&program.stmts);
    fn_names.retain(|n| !globals.contains(n));
    let mut bodies = Vec::new();

    let toplevel: Vec<&TypedStmt> = program
        .stmts
        .iter()
        .filter(|s| {
            !matches!(
                s.kind,
                TypedStmtKind::Function(_)
                    | TypedStmtKind::StructDecl { .. }
                    | TypedStmtKind::EnumDecl { .. }
                    | TypedStmtKind::Needs(_)
            )
        })
        .collect();
    bodies.push(build_toplevel(tt, &toplevel, program, &fn_names));

    for_each_fn_decl(&program.stmts, &mut |func, _parent| {
        bodies.push(build_function(tt, func, &fn_names));
    });

    BirProgram { bodies }
}

// no `_` arm and no `..` rest pattern below, so a new variant or field is a compile error here
pub fn for_each_fn_decl<F>(stmts: &[TypedStmt], f: &mut F)
where
    F: FnMut(&TypedFunction, Option<&str>),
{
    for stmt in stmts {
        fn_decls_in_stmt(stmt, None, f);
    }
}

fn fn_decls_in_stmt<F>(stmt: &TypedStmt, parent: Option<&str>, f: &mut F)
where
    F: FnMut(&TypedFunction, Option<&str>),
{
    match &stmt.kind {
        TypedStmtKind::Expression(expr) => fn_decls_in_expr(expr, parent, f),
        TypedStmtKind::Let {
            name: _,
            mutable: _,
            initializer,
            var_type: _,
            is_pub: _,
        } => fn_decls_in_expr(initializer, parent, f),
        TypedStmtKind::Block(stmts) => {
            for s in stmts {
                fn_decls_in_stmt(s, parent, f);
            }
        }
        TypedStmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            fn_decls_in_expr(condition, parent, f);
            fn_decls_in_stmt(then_branch, parent, f);
            if let Some(alt) = else_branch {
                fn_decls_in_stmt(alt, parent, f);
            }
        }
        TypedStmtKind::While { condition, body } => {
            fn_decls_in_expr(condition, parent, f);
            fn_decls_in_stmt(body, parent, f);
        }
        TypedStmtKind::For {
            iterator: _,
            start,
            end,
            inclusive: _,
            step,
            body,
        } => {
            fn_decls_in_expr(start, parent, f);
            fn_decls_in_expr(end, parent, f);
            if let Some(step) = step.as_ref() {
                fn_decls_in_expr(step, parent, f);
            }
            fn_decls_in_stmt(body, parent, f);
        }
        TypedStmtKind::ForEach {
            iterator: _,
            iterable,
            elem_type: _,
            body,
        } => {
            fn_decls_in_expr(iterable, parent, f);
            fn_decls_in_stmt(body, parent, f);
        }
        TypedStmtKind::Return(value) => {
            if let Some(expr) = value {
                fn_decls_in_expr(expr, parent, f);
            }
        }
        TypedStmtKind::Break | TypedStmtKind::Continue => {}
        TypedStmtKind::Function(func) => {
            f(func, parent);
            for s in &func.body {
                fn_decls_in_stmt(s, Some(&func.name), f);
            }
        }
        TypedStmtKind::Needs(_) => {}
        TypedStmtKind::StructDecl {
            name: _,
            type_params: _,
            fields: _,
        } => {}
        TypedStmtKind::EnumDecl {
            name: _,
            type_params: _,
            variants: _,
        } => {}
    }
}

fn fn_decls_in_expr<F>(expr: &TypedExpr, parent: Option<&str>, f: &mut F)
where
    F: FnMut(&TypedFunction, Option<&str>),
{
    match &expr.kind {
        TypedExprKind::Int(_)
        | TypedExprKind::Float(_)
        | TypedExprKind::Bool(_)
        | TypedExprKind::String(_)
        | TypedExprKind::Null
        | TypedExprKind::Identifier(_) => {}
        TypedExprKind::FmtString(parts) => {
            for part in parts {
                fn_decls_in_fmt_part(part, parent, f);
            }
        }
        TypedExprKind::Binary { left, op: _, right } => {
            fn_decls_in_expr(left, parent, f);
            fn_decls_in_expr(right, parent, f);
        }
        TypedExprKind::Unary { op: _, operand } => fn_decls_in_expr(operand, parent, f),
        TypedExprKind::And { left, right } | TypedExprKind::Or { left, right } => {
            fn_decls_in_expr(left, parent, f);
            fn_decls_in_expr(right, parent, f);
        }
        TypedExprKind::Call { callee, args } => {
            fn_decls_in_expr(callee, parent, f);
            for arg in args {
                fn_decls_in_expr(arg, parent, f);
            }
        }
        TypedExprKind::Assign { name: _, value } => fn_decls_in_expr(value, parent, f),
        TypedExprKind::Grouping(inner) => fn_decls_in_expr(inner, parent, f),
        TypedExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            fn_decls_in_expr(condition, parent, f);
            fn_decls_in_expr(then_branch, parent, f);
            fn_decls_in_expr(else_branch, parent, f);
        }
        TypedExprKind::Lambda(inner) => fn_decls_in_expr(inner, parent, f),
        TypedExprKind::LambdaInner {
            params: _,
            return_type: _,
            body,
            captures: _,
        } => {
            for s in body {
                fn_decls_in_stmt(s, parent, f);
            }
        }
        TypedExprKind::Member { object, member: _ } => fn_decls_in_expr(object, parent, f),
        TypedExprKind::ArrayLiteral { elements } => {
            for e in elements {
                fn_decls_in_expr(e, parent, f);
            }
        }
        TypedExprKind::ArraySized { size, fill_value } => {
            fn_decls_in_expr(size, parent, f);
            if let Some(fill) = fill_value {
                fn_decls_in_expr(fill, parent, f);
            }
        }
        TypedExprKind::VecLiteral {
            element_type: _,
            elements,
        } => {
            for e in elements {
                fn_decls_in_expr(e, parent, f);
            }
        }
        TypedExprKind::Index { object, index } => {
            fn_decls_in_expr(object, parent, f);
            fn_decls_in_expr(index, parent, f);
        }
        TypedExprKind::IndexAssign {
            object,
            index,
            value,
        } => {
            fn_decls_in_expr(object, parent, f);
            fn_decls_in_expr(index, parent, f);
            fn_decls_in_expr(value, parent, f);
        }
        TypedExprKind::FieldAssign {
            object,
            field: _,
            value,
        } => {
            fn_decls_in_expr(object, parent, f);
            fn_decls_in_expr(value, parent, f);
        }
        TypedExprKind::Range {
            start,
            end,
            inclusive: _,
        } => {
            if let Some(start) = start {
                fn_decls_in_expr(start, parent, f);
            }
            if let Some(end) = end {
                fn_decls_in_expr(end, parent, f);
            }
        }
        TypedExprKind::Slice { object, range } => {
            fn_decls_in_expr(object, parent, f);
            fn_decls_in_expr(range, parent, f);
        }
        TypedExprKind::Reference {
            mutable: _,
            operand,
        } => fn_decls_in_expr(operand, parent, f),
        TypedExprKind::Deref(inner) => fn_decls_in_expr(inner, parent, f),
        TypedExprKind::DerefAssign { target, value } => {
            fn_decls_in_expr(target, parent, f);
            fn_decls_in_expr(value, parent, f);
        }
        TypedExprKind::StructLiteral { name: _, fields } => {
            for (_, value) in fields {
                fn_decls_in_expr(value, parent, f);
            }
        }
        TypedExprKind::Cast { expr, target: _ } => fn_decls_in_expr(expr, parent, f),
        TypedExprKind::EnumVariant {
            enum_name: _,
            variant: _,
            tag: _,
            args,
        } => {
            for arg in args {
                fn_decls_in_expr(arg, parent, f);
            }
        }
        TypedExprKind::Match { scrutinee, arms } => {
            fn_decls_in_expr(scrutinee, parent, f);
            for arm in arms {
                fn_decls_in_expr(&arm.body, parent, f);
            }
        }
        TypedExprKind::ResultAssert {
            scrutinee,
            ok_tag: _,
            payload_ty: _,
            on_err: _,
        } => fn_decls_in_expr(scrutinee, parent, f),
        TypedExprKind::Block { stmts, tail } => {
            for s in stmts {
                fn_decls_in_stmt(s, parent, f);
            }
            fn_decls_in_expr(tail, parent, f);
        }
    }
}

fn fn_decls_in_fmt_part<F>(part: &TypedFmtStringPart, parent: Option<&str>, f: &mut F)
where
    F: FnMut(&TypedFunction, Option<&str>),
{
    match part {
        TypedFmtStringPart::Literal(_) | TypedFmtStringPart::Placeholder => {}
        TypedFmtStringPart::Expr(expr) => fn_decls_in_expr(expr, parent, f),
    }
}

fn gather_fn_names(stmts: &[TypedStmt]) -> HashSet<String> {
    let mut names = HashSet::new();
    for_each_fn_decl(stmts, &mut |func, _parent| {
        names.insert(func.name.clone());
    });
    names
}

fn gather_global_names(stmts: &[TypedStmt]) -> HashSet<String> {
    let mut names = HashSet::new();
    for stmt in stmts {
        if let TypedStmtKind::Let { name, .. } = &stmt.kind {
            names.insert(name.clone());
        }
    }
    names
}

fn build_toplevel(
    tt: &TypeTable,
    stmts: &[&TypedStmt],
    program: &TypedProgram,
    fn_names: &HashSet<String>,
) -> BirBody {
    let span = program
        .stmts
        .first()
        .map(|s| s.span)
        .unwrap_or(Span::dummy());
    let mut b = BodyBuilder::new(
        tt,
        "__toplevel".to_string(),
        span,
        true,
        fn_names,
        InferType::Null,
    );
    (b.intrinsic_effects, b.managed_witness) = effects::intrinsic_effects(&program.stmts, &[], tt);
    b.open_scope(span);
    for stmt in stmts {
        b.build_stmt(stmt);
    }
    b.close_scope();
    b.finish()
}

fn build_function(tt: &TypeTable, func: &TypedFunction, fn_names: &HashSet<String>) -> BirBody {
    let mut b = BodyBuilder::new(
        tt,
        func.name.clone(),
        func.span,
        false,
        fn_names,
        func.return_type.clone(),
    );
    (b.intrinsic_effects, b.managed_witness) =
        effects::intrinsic_effects(&func.body, &func.params, tt);
    b.declared_nogc = func.declared_nogc;
    b.open_scope(func.span);
    for p in &func.params {
        b.new_named(&p.name, p.ty.clone(), p.span, p.mutable);
    }
    b.arg_count = b.locals.len();
    for stmt in &func.body {
        b.build_stmt(stmt);
    }
    b.close_scope();
    b.finish()
}

struct ScopeFrame {
    span: Span,
    name_len: usize,
    affine: Vec<BirLocalId>,
}

struct BodyBuilder<'a> {
    tt: &'a TypeTable,
    name: String,
    span: Span,
    is_toplevel: bool,
    fn_names: &'a HashSet<String>,
    return_type: InferType,
    locals: Vec<BirLocal>,
    arg_count: usize,
    blocks: Vec<BirBlock>,
    cur_id: BirBlockId,
    cur_stmts: Vec<BirStmt>,
    cur_open: bool,
    next_block: u32,
    name_map: Vec<(String, BirLocalId)>,
    scopes: Vec<ScopeFrame>,
    loop_stack: Vec<(BirBlockId, BirBlockId)>,
    scope_exits: Vec<ScopeExit>,
    returns: Vec<ReturnPoint>,
    reassigns: Vec<Reassign>,
    scope_deaths: Vec<ScopeDeath>,
    build_errors: Vec<BirDiagnostic>,
    intrinsic_effects: EffectSet,
    managed_witness: Option<(Span, String)>,
    declared_nogc: bool,
}

impl<'a> BodyBuilder<'a> {
    fn new(
        tt: &'a TypeTable,
        name: String,
        span: Span,
        is_toplevel: bool,
        fn_names: &'a HashSet<String>,
        return_type: InferType,
    ) -> Self {
        Self {
            tt,
            name,
            span,
            is_toplevel,
            fn_names,
            return_type,
            locals: Vec::new(),
            arg_count: 0,
            blocks: Vec::new(),
            cur_id: BirBlockId(0),
            cur_stmts: Vec::new(),
            cur_open: true,
            next_block: 1,
            name_map: Vec::new(),
            scopes: Vec::new(),
            loop_stack: Vec::new(),
            scope_exits: Vec::new(),
            returns: Vec::new(),
            reassigns: Vec::new(),
            scope_deaths: Vec::new(),
            build_errors: Vec::new(),
            intrinsic_effects: EffectSet::EMPTY,
            managed_witness: None,
            declared_nogc: false,
        }
    }

    fn finish(mut self) -> BirBody {
        if self.cur_open {
            self.seal(BirTerminator::Return(None), self.span);
        }
        BirBody {
            name: self.name,
            locals: self.locals,
            arg_count: self.arg_count,
            blocks: self.blocks,
            entry: BirBlockId(0),
            span: self.span,
            scope_exits: self.scope_exits,
            returns: self.returns,
            reassigns: self.reassigns,
            is_toplevel: self.is_toplevel,
            return_type: self.return_type,
            build_errors: self.build_errors,
            scope_deaths: self.scope_deaths,
            intrinsic_effects: self.intrinsic_effects,
            managed_witness: self.managed_witness,
            declared_nogc: self.declared_nogc,
        }
    }

    fn recover_callee(&self, callee: &TypedExpr) -> Option<String> {
        if let TypedExprKind::Identifier(name) = &callee.kind {
            if self.lookup(name).is_none() && self.fn_names.contains(name) {
                return Some(name.clone());
            }
        }
        None
    }

    fn indirect_nogc_callee(&self, recovered: &Option<String>, callee: &TypedExpr) -> bool {
        if recovered.is_some() {
            return false;
        }
        if let TypedExprKind::Identifier(name) = &callee.kind {
            if let Some(id) = self.lookup(name) {
                let idx = id.0 as usize;
                if idx < self.arg_count {
                    return matches!(&self.locals[idx].ty, InferType::Function { nogc: true, .. });
                }
            }
        }
        false
    }

    // a reference reaching a container through a projected store escapes the aggregate guard,
    fn reject_projected_ref_store(&mut self, value: &TypedExpr) {
        if is_ref_ty(&value.ty) {
            self.build_errors.push(BirDiagnostic::new(
                "E0724",
                "[escape]",
                value.span,
                "[escape] references stored into aggregate containers are not supported in Run 1"
                    .to_string(),
            ));
        }
    }

    // a closure becomes an opaque const with no bir trace, so a ref capture is rejected here
    fn reject_ref_captures(&mut self, captures: &[(String, InferType)], span: Span) {
        if captures.iter().any(|(_, ty)| is_ref_ty(ty)) {
            self.build_errors.push(BirDiagnostic::new(
                "E0725",
                "[escape]",
                span,
                "[escape] a closure that captures a reference is not supported in Run 1"
                    .to_string(),
            ));
        }
    }

    fn cat(&self, ty: &InferType) -> Category {
        category(ty, self.tt)
    }

    fn new_block_id(&mut self) -> BirBlockId {
        let id = BirBlockId(self.next_block);
        self.next_block += 1;
        id
    }

    fn seal(&mut self, term: BirTerminator, term_span: Span) {
        let stmts = std::mem::take(&mut self.cur_stmts);
        self.blocks.push(BirBlock {
            id: self.cur_id,
            stmts,
            term,
            term_span,
        });
        self.cur_open = false;
    }

    fn start(&mut self, id: BirBlockId) {
        self.cur_id = id;
        self.cur_stmts = Vec::new();
        self.cur_open = true;
    }

    fn push(&mut self, kind: BirStmtKind, span: Span) {
        if self.cur_open {
            self.cur_stmts.push(BirStmt { kind, span });
        }
    }

    fn lookup(&self, name: &str) -> Option<BirLocalId> {
        self.name_map
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
    }

    fn new_temp(&mut self, ty: InferType, span: Span) -> BirLocalId {
        let id = BirLocalId(self.locals.len() as u32);
        let category = self.cat(&ty);
        self.locals.push(BirLocal {
            id,
            name: None,
            ty,
            category,
            decl_span: span,
            mutable: false,
        });
        id
    }

    fn new_named(
        &mut self,
        name: &str,
        ty: InferType,
        decl_span: Span,
        mutable: bool,
    ) -> BirLocalId {
        let id = BirLocalId(self.locals.len() as u32);
        let category = self.cat(&ty);
        self.locals.push(BirLocal {
            id,
            name: Some(name.to_string()),
            ty,
            category,
            decl_span,
            mutable,
        });
        self.name_map.push((name.to_string(), id));
        if category == Category::Affine {
            if let Some(frame) = self.scopes.last_mut() {
                frame.affine.push(id);
            }
        }
        id
    }

    fn open_scope(&mut self, span: Span) {
        self.scopes.push(ScopeFrame {
            span,
            name_len: self.name_map.len(),
            affine: Vec::new(),
        });
    }

    fn close_scope(&mut self) {
        let frame = self.scopes.pop().expect("balanced scopes");
        // fallthrough (open) block reaches this point. exit_index is captured before the
        if self.cur_open {
            let exit_index = self.cur_stmts.len();
            // every named non-parameter local declared here dies at this exit (the escape pass
            let dying: Vec<BirLocalId> = self.name_map[frame.name_len..]
                .iter()
                .map(|(_, id)| *id)
                .filter(|id| (id.0 as usize) >= self.arg_count)
                .collect();
            if !dying.is_empty() {
                self.scope_deaths.push(ScopeDeath {
                    block: self.cur_id,
                    index: exit_index,
                    locals: dying,
                    scope_span: frame.span,
                });
            }
            if !frame.affine.is_empty() {
                self.scope_exits.push(ScopeExit {
                    scope_span: frame.span,
                    exit_block: self.cur_id,
                    exit_index,
                    locals: frame.affine.clone(),
                });
                for id in &frame.affine {
                    self.push(BirStmtKind::StorageDead(*id), frame.span);
                }
            }
        }
        self.name_map.truncate(frame.name_len);
    }

    fn in_scope_affine(&self) -> Vec<BirLocalId> {
        self.scopes
            .iter()
            .flat_map(|f| f.affine.iter().copied())
            .collect()
    }

    fn emit_to_temp(&mut self, rvalue: BirRvalue, ty: InferType, span: Span) -> BirOperand {
        let temp = self.new_temp(ty, span);
        self.push(
            BirStmtKind::Assign {
                dest: BirPlace {
                    local: temp,
                    proj: Vec::new(),
                },
                rvalue,
            },
            span,
        );
        BirOperand::Copy(BirPlace {
            local: temp,
            proj: Vec::new(),
        })
    }

    fn place_of(&mut self, expr: &TypedExpr) -> Option<BirPlace> {
        match &expr.kind {
            TypedExprKind::Identifier(name) => self.lookup(name).map(|local| BirPlace {
                local,
                proj: Vec::new(),
            }),
            TypedExprKind::Member { object, member } => {
                let mut base = self.place_of(object)?;
                base.proj.push(BirProjection::Field(member.clone()));
                Some(base)
            }
            TypedExprKind::Index { object, index } => {
                let _ = self.build_operand(index);
                let mut base = self.place_of(object)?;
                base.proj.push(BirProjection::Index);
                Some(base)
            }
            TypedExprKind::Deref(inner) => {
                let mut base = self.place_of(inner)?;
                base.proj.push(BirProjection::Deref);
                Some(base)
            }
            TypedExprKind::Grouping(inner) => self.place_of(inner),
            _ => None,
        }
    }

    fn build_operand(&mut self, expr: &TypedExpr) -> BirOperand {
        let span = expr.span;
        match &expr.kind {
            TypedExprKind::Int(_)
            | TypedExprKind::Float(_)
            | TypedExprKind::Bool(_)
            | TypedExprKind::String(_)
            | TypedExprKind::FmtString(_)
            | TypedExprKind::Null => BirOperand::Const,

            TypedExprKind::Identifier(name) => match self.lookup(name) {
                Some(local) => {
                    let place = BirPlace {
                        local,
                        proj: Vec::new(),
                    };
                    if self.locals[local.0 as usize].category == Category::Affine {
                        BirOperand::Move(place)
                    } else {
                        BirOperand::Copy(place)
                    }
                }
                None => BirOperand::Const,
            },

            TypedExprKind::Member { .. }
            | TypedExprKind::Index { .. }
            | TypedExprKind::Deref(_) => match self.place_of(expr) {
                Some(place) => BirOperand::Copy(place),
                None => BirOperand::Const,
            },

            TypedExprKind::Grouping(inner) => self.build_operand(inner),
            TypedExprKind::Cast { expr: inner, .. } => {
                let op = self.build_operand(inner);
                self.emit_to_temp(BirRvalue::UnOp(op), expr.ty.clone(), span)
            }

            TypedExprKind::Binary { left, right, .. } => {
                let l = self.build_operand(left);
                let r = self.build_operand(right);
                self.emit_to_temp(BirRvalue::BinOp(l, r), expr.ty.clone(), span)
            }
            TypedExprKind::Unary { operand, .. } => {
                let o = self.build_operand(operand);
                self.emit_to_temp(BirRvalue::UnOp(o), expr.ty.clone(), span)
            }

            TypedExprKind::And { left, right } => self.build_short_circuit(left, right, true, expr),
            TypedExprKind::Or { left, right } => self.build_short_circuit(left, right, false, expr),

            TypedExprKind::Call { callee, args } => {
                let recovered = self.recover_callee(callee);
                let indirect_nogc = self.indirect_nogc_callee(&recovered, callee);
                let ops: Vec<BirOperand> = args.iter().map(|a| self.build_operand(a)).collect();
                self.emit_to_temp(
                    BirRvalue::Call {
                        callee: recovered,
                        args: ops,
                        indirect_nogc,
                    },
                    expr.ty.clone(),
                    span,
                )
            }

            TypedExprKind::StructLiteral { fields, .. } => {
                let ops: Vec<BirOperand> =
                    fields.iter().map(|(_, v)| self.build_operand(v)).collect();
                self.emit_to_temp(BirRvalue::Aggregate(ops), expr.ty.clone(), span)
            }
            TypedExprKind::EnumVariant {
                enum_name,
                variant,
                args,
                ..
            } => {
                // row 6: the sole run-1 vec mutator writes its receiver, so a live element borrow
                if enum_name.as_str() == "Vec" && variant.as_str() == "push" {
                    if let Some((recv, rest)) = args.split_first() {
                        match self.build_operand(recv) {
                            BirOperand::Copy(dest) => {
                                let ops = rest.iter().map(|a| self.build_operand(a)).collect();
                                self.push(
                                    BirStmtKind::Assign {
                                        dest,
                                        rvalue: BirRvalue::Aggregate(ops),
                                    },
                                    span,
                                );
                                return BirOperand::Const;
                            }
                            recv_op => {
                                // a non-place receiver (vec::push(make(), x)) has no outstanding
                                // borrow, so the reused single operand is read-modeled, sound either way
                                let mut ops = vec![recv_op];
                                ops.extend(rest.iter().map(|a| self.build_operand(a)));
                                return self.emit_to_temp(
                                    BirRvalue::Aggregate(ops),
                                    expr.ty.clone(),
                                    span,
                                );
                            }
                        }
                    }
                }
                let ops: Vec<BirOperand> = args.iter().map(|a| self.build_operand(a)).collect();
                self.emit_to_temp(BirRvalue::Aggregate(ops), expr.ty.clone(), span)
            }
            TypedExprKind::ArrayLiteral { elements }
            | TypedExprKind::VecLiteral { elements, .. } => {
                let ops: Vec<BirOperand> = elements.iter().map(|e| self.build_operand(e)).collect();
                self.emit_to_temp(BirRvalue::Aggregate(ops), expr.ty.clone(), span)
            }
            TypedExprKind::ArraySized { size, fill_value } => {
                let _ = self.build_operand(size);
                if let Some(fv) = fill_value {
                    let _ = self.build_operand(fv);
                }
                self.emit_to_temp(BirRvalue::Aggregate(Vec::new()), expr.ty.clone(), span)
            }

            TypedExprKind::Reference { mutable, operand } => {
                // a reference to a reference lets a loan escape via a deref-copy that whole-local provenance misses
                if is_ref_ty(&operand.ty) {
                    self.build_errors.push(BirDiagnostic::new(
                        "E0726",
                        "[escape]",
                        span,
                        "[escape] references to references are not supported in Run 1".to_string(),
                    ));
                }
                match self.place_of(operand) {
                    Some(place) => self.emit_to_temp(
                        BirRvalue::Ref {
                            place,
                            mutable: *mutable,
                        },
                        expr.ty.clone(),
                        span,
                    ),
                    None => {
                        let op = self.build_operand(operand);
                        self.emit_to_temp(BirRvalue::Use(op), expr.ty.clone(), span)
                    }
                }
            }

            // a slice is a borrow of its base: no e0726 here, a slice of a slice is a kept form
            TypedExprKind::Slice { object, range } => {
                let _ = self.build_operand(range);
                match self.place_of(object) {
                    Some(place) => self.emit_to_temp(
                        BirRvalue::Ref {
                            place,
                            mutable: false,
                        },
                        expr.ty.clone(),
                        span,
                    ),
                    None => {
                        let op = self.build_operand(object);
                        self.emit_to_temp(BirRvalue::Use(op), expr.ty.clone(), span)
                    }
                }
            }
            TypedExprKind::Range { start, end, .. } => {
                if let Some(s) = start {
                    let _ = self.build_operand(s);
                }
                if let Some(e) = end {
                    let _ = self.build_operand(e);
                }
                BirOperand::Const
            }

            TypedExprKind::Assign { name, value } => {
                self.build_assign(name, value, expr.span);
                match self.lookup(name) {
                    Some(local) => BirOperand::Copy(BirPlace {
                        local,
                        proj: Vec::new(),
                    }),
                    None => BirOperand::Const,
                }
            }
            TypedExprKind::FieldAssign {
                object,
                field,
                value,
            } => {
                self.build_field_assign(object, field, value, expr.span);
                BirOperand::Const
            }
            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
                self.reject_projected_ref_store(value);
                let v = self.build_operand(value);
                let _ = self.build_operand(index);
                if let Some(mut place) = self.place_of(object) {
                    place.proj.push(BirProjection::Index);
                    self.push(
                        BirStmtKind::Assign {
                            dest: place,
                            rvalue: BirRvalue::Use(v),
                        },
                        span,
                    );
                }
                BirOperand::Const
            }
            TypedExprKind::DerefAssign { target, value } => {
                self.reject_projected_ref_store(value);
                let v = self.build_operand(value);
                if let Some(mut place) = self.place_of(target) {
                    place.proj.push(BirProjection::Deref);
                    self.push(
                        BirStmtKind::Assign {
                            dest: place,
                            rvalue: BirRvalue::Use(v),
                        },
                        span,
                    );
                }
                BirOperand::Const
            }

            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.build_if_expr(condition, then_branch, Some(else_branch), expr),
            TypedExprKind::Match { scrutinee, arms } => {
                self.build_match_expr(scrutinee, arms, expr)
            }
            TypedExprKind::Block { stmts, tail } => self.build_block_expr(stmts, tail),

            TypedExprKind::ResultAssert { scrutinee, .. } => {
                // build the scrutinee for its reads; the err arm seals a divergence, modeled
                let op = self.build_operand(scrutinee);
                self.emit_to_temp(BirRvalue::Use(op), expr.ty.clone(), span)
            }

            TypedExprKind::LambdaInner { captures, .. } => {
                self.reject_ref_captures(captures, span);
                BirOperand::Const
            }
            TypedExprKind::Lambda(inner) => {
                if let TypedExprKind::LambdaInner { captures, .. } = &inner.kind {
                    self.reject_ref_captures(captures, span);
                }
                BirOperand::Const
            }
        }
    }

    fn build_short_circuit(
        &mut self,
        left: &TypedExpr,
        right: &TypedExpr,
        is_and: bool,
        expr: &TypedExpr,
    ) -> BirOperand {
        let cond = self.build_operand(left);
        let rhs_id = self.new_block_id();
        let merge_id = self.new_block_id();
        let targets = if is_and {
            vec![rhs_id, merge_id]
        } else {
            vec![merge_id, rhs_id]
        };
        self.seal(
            BirTerminator::Branch {
                discr: cond,
                targets,
            },
            left.span,
        );
        self.start(rhs_id);
        let _ = self.build_operand(right);
        if self.cur_open {
            self.seal(BirTerminator::Goto(merge_id), expr.span);
        }
        self.start(merge_id);
        let _ = expr;
        BirOperand::Const
    }

    fn build_if_expr(
        &mut self,
        condition: &TypedExpr,
        then_branch: &TypedExpr,
        else_branch: Option<&TypedExpr>,
        expr: &TypedExpr,
    ) -> BirOperand {
        let cond = self.build_operand(condition);
        let then_id = self.new_block_id();
        let merge_id = self.new_block_id();
        let else_id = if else_branch.is_some() {
            self.new_block_id()
        } else {
            merge_id
        };
        self.seal(
            BirTerminator::Branch {
                discr: cond,
                targets: vec![then_id, else_id],
            },
            condition.span,
        );

        self.start(then_id);
        let _ = self.build_operand(then_branch);
        if self.cur_open {
            self.seal(BirTerminator::Goto(merge_id), then_branch.span);
        }
        if let Some(else_br) = else_branch {
            self.start(else_id);
            let _ = self.build_operand(else_br);
            if self.cur_open {
                self.seal(BirTerminator::Goto(merge_id), else_br.span);
            }
        }
        self.start(merge_id);
        let _ = expr;
        BirOperand::Const
    }

    fn build_match_expr(
        &mut self,
        scrutinee: &TypedExpr,
        arms: &[TypedMatchArm],
        expr: &TypedExpr,
    ) -> BirOperand {
        let discr = self.build_operand(scrutinee);
        let merge_id = self.new_block_id();
        let arm_ids: Vec<BirBlockId> = arms.iter().map(|_| self.new_block_id()).collect();
        let mut targets = arm_ids.clone();
        if targets.is_empty() {
            targets.push(merge_id);
        }
        self.seal(BirTerminator::Branch { discr, targets }, scrutinee.span);
        for (arm, id) in arms.iter().zip(arm_ids.iter()) {
            self.start(*id);
            self.open_scope(arm.body.span);
            let _ = self.build_operand(&arm.body);
            self.close_scope();
            if self.cur_open {
                self.seal(BirTerminator::Goto(merge_id), arm.body.span);
            }
        }
        self.start(merge_id);
        let _ = expr;
        BirOperand::Const
    }

    fn build_block_expr(&mut self, stmts: &[TypedStmt], tail: &TypedExpr) -> BirOperand {
        self.open_scope(tail.span);
        for stmt in stmts {
            self.build_stmt(stmt);
        }
        let op = self.build_operand(tail);
        let ty = tail.ty.clone();
        let out = self.emit_to_temp(BirRvalue::Use(op), ty, tail.span);
        self.close_scope();
        out
    }

    fn build_assign(&mut self, name: &str, value: &TypedExpr, span: Span) {
        let v = self.build_operand(value);
        if let Some(local) = self.lookup(name) {
            let is_affine = self.locals[local.0 as usize].category == Category::Affine;
            if is_affine && self.cur_open {
                self.reassigns.push(Reassign {
                    span,
                    block: self.cur_id,
                    stmt_index: self.cur_stmts.len(),
                    local,
                });
            }
            self.push(
                BirStmtKind::Assign {
                    dest: BirPlace {
                        local,
                        proj: Vec::new(),
                    },
                    rvalue: BirRvalue::Use(v),
                },
                span,
            );
        }
    }

    fn build_field_assign(
        &mut self,
        object: &TypedExpr,
        field: &str,
        value: &TypedExpr,
        span: Span,
    ) {
        self.reject_projected_ref_store(value);
        let v = self.build_operand(value);
        if let Some(mut place) = self.place_of(object) {
            place.proj.push(BirProjection::Field(field.to_string()));
            self.push(
                BirStmtKind::Assign {
                    dest: place,
                    rvalue: BirRvalue::Use(v),
                },
                span,
            );
        }
    }

    fn build_stmt(&mut self, stmt: &TypedStmt) {
        let span = stmt.span;
        match &stmt.kind {
            TypedStmtKind::Expression(e) => {
                self.build_effect(e);
            }
            TypedStmtKind::Let {
                name,
                mutable,
                initializer,
                var_type,
                ..
            } => {
                let v = self.build_operand(initializer);
                let local = self.new_named(name, var_type.clone(), span, *mutable);
                self.push(BirStmtKind::StorageLive(local), span);
                self.push(
                    BirStmtKind::Assign {
                        dest: BirPlace {
                            local,
                            proj: Vec::new(),
                        },
                        rvalue: BirRvalue::Use(v),
                    },
                    span,
                );
            }
            TypedStmtKind::Block(stmts) => {
                self.open_scope(span);
                for s in stmts {
                    self.build_stmt(s);
                }
                self.close_scope();
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.build_if_stmt(condition, then_branch, else_branch.as_deref());
            }
            TypedStmtKind::While { condition, body } => {
                self.build_while(condition, body);
            }
            TypedStmtKind::For {
                iterator,
                start,
                end,
                step,
                body,
                ..
            } => {
                self.build_for(iterator, start, end, step.as_ref().as_ref(), body);
            }
            TypedStmtKind::ForEach {
                iterator,
                iterable,
                elem_type,
                body,
            } => {
                self.build_foreach(iterator, iterable, elem_type, body);
            }
            TypedStmtKind::Return(val) => {
                let op = val.as_ref().map(|e| self.build_operand(e));
                if self.cur_open {
                    self.returns.push(ReturnPoint {
                        span,
                        block: self.cur_id,
                        in_scope: self.in_scope_affine(),
                    });
                    self.seal(BirTerminator::Return(op), span);
                }
            }
            TypedStmtKind::Break => {
                if let Some((_, exit)) = self.loop_stack.last().copied() {
                    self.seal(BirTerminator::Goto(exit), span);
                } else {
                    self.seal(BirTerminator::Unreachable, span);
                }
            }
            TypedStmtKind::Continue => {
                if let Some((header, _)) = self.loop_stack.last().copied() {
                    self.seal(BirTerminator::Goto(header), span);
                } else {
                    self.seal(BirTerminator::Unreachable, span);
                }
            }
            TypedStmtKind::Function(_)
            | TypedStmtKind::Needs(_)
            | TypedStmtKind::StructDecl { .. }
            | TypedStmtKind::EnumDecl { .. } => {}
        }
    }

    fn build_effect(&mut self, expr: &TypedExpr) {
        let span = expr.span;
        match &expr.kind {
            TypedExprKind::Call { callee, args } => {
                let recovered = self.recover_callee(callee);
                let indirect_nogc = self.indirect_nogc_callee(&recovered, callee);
                let ops: Vec<BirOperand> = args.iter().map(|a| self.build_operand(a)).collect();
                let temp = self.new_temp(expr.ty.clone(), span);
                self.push(
                    BirStmtKind::Assign {
                        dest: BirPlace {
                            local: temp,
                            proj: Vec::new(),
                        },
                        rvalue: BirRvalue::Call {
                            callee: recovered,
                            args: ops,
                            indirect_nogc,
                        },
                    },
                    span,
                );
            }
            TypedExprKind::Assign { name, value } => self.build_assign(name, value, span),
            TypedExprKind::FieldAssign {
                object,
                field,
                value,
            } => self.build_field_assign(object, field, value, span),
            TypedExprKind::IndexAssign { .. } | TypedExprKind::DerefAssign { .. } => {
                let _ = self.build_operand(expr);
            }
            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                let _ = self.build_if_expr(condition, then_branch, Some(else_branch), expr);
            }
            TypedExprKind::Match { scrutinee, arms } => {
                let _ = self.build_match_expr(scrutinee, arms, expr);
            }
            TypedExprKind::Block { stmts, tail } => {
                let _ = self.build_block_expr(stmts, tail);
            }
            _ => {
                let _ = self.build_operand(expr);
            }
        }
    }

    fn build_if_stmt(
        &mut self,
        condition: &TypedExpr,
        then_branch: &TypedStmt,
        else_branch: Option<&TypedStmt>,
    ) {
        let cond = self.build_operand(condition);
        let then_id = self.new_block_id();
        let merge_id = self.new_block_id();
        let else_id = if else_branch.is_some() {
            self.new_block_id()
        } else {
            merge_id
        };
        self.seal(
            BirTerminator::Branch {
                discr: cond,
                targets: vec![then_id, else_id],
            },
            condition.span,
        );

        self.start(then_id);
        self.build_stmt(then_branch);
        if self.cur_open {
            self.seal(BirTerminator::Goto(merge_id), then_branch.span);
        }
        if let Some(else_br) = else_branch {
            self.start(else_id);
            self.build_stmt(else_br);
            if self.cur_open {
                self.seal(BirTerminator::Goto(merge_id), else_br.span);
            }
        }
        self.start(merge_id);
    }

    fn build_while(&mut self, condition: &TypedExpr, body: &TypedStmt) {
        let header_id = self.new_block_id();
        let body_id = self.new_block_id();
        let exit_id = self.new_block_id();
        self.seal(BirTerminator::Goto(header_id), condition.span);

        self.start(header_id);
        let cond = self.build_operand(condition);
        self.seal(
            BirTerminator::Branch {
                discr: cond,
                targets: vec![body_id, exit_id],
            },
            condition.span,
        );

        self.loop_stack.push((header_id, exit_id));
        self.start(body_id);
        self.build_stmt(body);
        if self.cur_open {
            self.seal(BirTerminator::Goto(header_id), body.span);
        }
        self.loop_stack.pop();
        self.start(exit_id);
    }

    fn build_for(
        &mut self,
        iterator: &str,
        start: &TypedExpr,
        end: &TypedExpr,
        step: Option<&TypedExpr>,
        body: &TypedStmt,
    ) {
        let _ = self.build_operand(start);
        let _ = self.build_operand(end);
        if let Some(s) = step {
            let _ = self.build_operand(s);
        }
        let header_id = self.new_block_id();
        let body_id = self.new_block_id();
        let exit_id = self.new_block_id();
        self.seal(BirTerminator::Goto(header_id), body.span);
        self.start(header_id);
        self.seal(
            BirTerminator::Branch {
                discr: BirOperand::Const,
                targets: vec![body_id, exit_id],
            },
            body.span,
        );
        self.loop_stack.push((header_id, exit_id));
        self.start(body_id);
        self.open_scope(body.span);
        self.new_named(iterator, InferType::I64, body.span, true);
        self.build_stmt(body);
        self.close_scope();
        if self.cur_open {
            self.seal(BirTerminator::Goto(header_id), body.span);
        }
        self.loop_stack.pop();
        self.start(exit_id);
    }

    fn build_foreach(
        &mut self,
        iterator: &str,
        iterable: &TypedExpr,
        elem_type: &InferType,
        body: &TypedStmt,
    ) {
        let _ = self.build_operand(iterable);
        let header_id = self.new_block_id();
        let body_id = self.new_block_id();
        let exit_id = self.new_block_id();
        self.seal(BirTerminator::Goto(header_id), body.span);
        self.start(header_id);
        self.seal(
            BirTerminator::Branch {
                discr: BirOperand::Const,
                targets: vec![body_id, exit_id],
            },
            body.span,
        );
        self.loop_stack.push((header_id, exit_id));
        self.start(body_id);
        self.open_scope(body.span);
        self.new_named(iterator, elem_type.clone(), body.span, true);
        self.build_stmt(body);
        self.close_scope();
        if self.cur_open {
            self.seal(BirTerminator::Goto(header_id), body.span);
        }
        self.loop_stack.pop();
        self.start(exit_id);
    }
}

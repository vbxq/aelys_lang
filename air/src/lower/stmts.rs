use super::LoweringContext;
use crate::*;
use aelys_sema::{InferType, TypedExprKind, TypedStmt, TypedStmtKind};

impl<'a> LoweringContext<'a> {
    pub(super) fn lower_body(&mut self, stmts: &[TypedStmt]) {
        // Save the current scope depth so inner `let` bindings don't leak out.
        let scope_depth = self.locals_by_name.len();
        for stmt in stmts {
            self.lower_stmt(stmt);
        }
        self.emit_scope_rc_releases(scope_depth);
        self.locals_by_name.truncate(scope_depth);
    }

    // passing the pointer as a use is what keeps copy_elim and dead_locals off the local
    pub(super) fn emit_rc_retain(&mut self, ptr: Operand, sp: Option<Span>) {
        self.emit(
            AirStmtKind::CallVoid {
                func: Callee::Named("__aelys_rc_retain".to_string()),
                args: vec![ptr],
            },
            sp,
        );
    }

    pub(super) fn emit_rc_release(&mut self, local: LocalId, sp: Option<Span>) {
        self.emit(
            AirStmtKind::CallVoid {
                func: Callee::Named("__aelys_rc_release".to_string()),
                args: vec![Operand::Copy(local)],
            },
            sp,
        );
    }

    fn emit_cow_buffer_call(&mut self, fn_name: &str, local: LocalId, sp: Option<Span>) {
        let vec_ty = self
            .local_air_type(local)
            .unwrap_or(AirType::Vec(Box::new(AirType::I64)));
        let addr = self.emit_rvalue_to_temp(
            AirType::Ptr(Box::new(vec_ty)),
            Rvalue::AddressOf(local),
            sp,
        );
        self.emit(
            AirStmtKind::CallVoid {
                func: Callee::Named(fn_name.to_string()),
                args: vec![addr],
            },
            sp,
        );
    }

    pub(super) fn emit_cow_retain(&mut self, local: LocalId, sp: Option<Span>) {
        self.emit_cow_buffer_call("__aelys_vec_retain", local, sp);
    }

    pub(super) fn emit_cow_release(&mut self, local: LocalId, sp: Option<Span>) {
        self.emit_cow_buffer_call("__aelys_vec_release", local, sp);
    }

    pub(super) fn emit_scope_rc_releases(&mut self, scope_depth: usize) {
        let has_rc = self.rc_locals.iter().any(|(_, d)| *d > scope_depth);
        let has_carrier = self.carrier_locals.iter().any(|c| c.depth > scope_depth);
        let has_cow = self.cow_locals.iter().any(|(_, d)| *d > scope_depth);
        if !has_rc && !has_carrier && !has_cow {
            return;
        }
        // the scope is already terminated, so anything emitted here would be dead code
        let terminated = self.last_block_is_terminated();
        let to_release: Vec<LocalId> = self
            .rc_locals
            .iter()
            .filter(|(_, d)| *d > scope_depth)
            .map(|(id, _)| *id)
            .collect();
        let carriers_to_release: Vec<(LocalId, AirType, Vec<crate::rc_paths::RcLeafPath>)> = self
            .carrier_locals
            .iter()
            .filter(|c| c.depth > scope_depth)
            .map(|c| (c.local, c.ty.clone(), c.paths.clone()))
            .collect();
        let cows_to_release: Vec<LocalId> = self
            .cow_locals
            .iter()
            .filter(|(_, d)| *d > scope_depth)
            .map(|(id, _)| *id)
            .collect();
        if !terminated {
            for id in &to_release {
                self.emit_rc_release(*id, None);
            }
            for (local, ty, paths) in &carriers_to_release {
                self.emit_carrier_field_releases(*local, ty, paths, None);
            }
            for id in &cows_to_release {
                self.emit_cow_release(*id, None);
            }
        }
        self.rc_locals.retain(|(_, d)| *d <= scope_depth);
        self.carrier_locals.retain(|c| c.depth <= scope_depth);
        self.cow_locals.retain(|(_, d)| *d <= scope_depth);
    }

    // the returned value keeps its count, it travels to the caller
    pub(super) fn emit_rc_releases_for_return(&mut self, returned: Option<&Operand>) {
        let escaped: Option<LocalId> = match returned {
            Some(Operand::Copy(id) | Operand::Move(id)) => Some(*id),
            _ => None,
        };
        let to_release: Vec<LocalId> = self
            .rc_locals
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| Some(*id) != escaped)
            .collect();
        for id in to_release {
            self.emit_rc_release(id, None);
        }
        let carriers_to_release: Vec<(LocalId, AirType, Vec<crate::rc_paths::RcLeafPath>)> = self
            .carrier_locals
            .iter()
            .filter(|c| Some(c.local) != escaped)
            .map(|c| (c.local, c.ty.clone(), c.paths.clone()))
            .collect();
        for (local, ty, paths) in &carriers_to_release {
            self.emit_carrier_field_releases(*local, ty, paths, None);
        }
        let cows_to_release: Vec<LocalId> = self
            .cow_locals
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| Some(*id) != escaped)
            .collect();
        for id in cows_to_release {
            self.emit_cow_release(id, None);
        }
    }

    pub(super) fn emit_param_cow_releases_on_fallthrough(&mut self) {
        if !self.cow_locals.iter().any(|(_, d)| *d == 0) {
            return;
        }
        if !self.last_block_is_terminated() {
            let to_release: Vec<LocalId> = self
                .cow_locals
                .iter()
                .filter(|(_, d)| *d == 0)
                .map(|(id, _)| *id)
                .collect();
            for id in to_release {
                self.emit_cow_release(id, None);
            }
        }
        self.cow_locals.retain(|(_, d)| *d != 0);
    }

    pub(super) fn rc_live_below(&self, threshold: usize) -> bool {
        self.rc_locals.iter().any(|(_, d)| *d > threshold)
            || self.carrier_locals.iter().any(|c| c.depth > threshold)
    }

    pub(super) fn emit_load_rc_leaf(
        &mut self,
        base: Operand,
        base_ty: &AirType,
        path: &crate::rc_paths::RcLeafPath,
        sp: Option<Span>,
    ) -> Operand {
        use crate::rc_paths::RcPathStep;
        let mut cur = base;
        let mut cur_ty = base_ty.clone();
        for step in &path.steps {
            match step {
                RcPathStep::Field(field) => {
                    let field_ty = self.air_struct_field_type(&cur_ty, field);
                    cur = self.emit_rvalue_to_temp(
                        field_ty.clone(),
                        Rvalue::FieldAccess {
                            base: cur,
                            field: field.clone(),
                        },
                        sp,
                    );
                    cur_ty = field_ty;
                }
                RcPathStep::EnumPayload {
                    enum_name,
                    tag,
                    field_index,
                } => {
                    let payload_ty =
                        self.air_enum_payload_type(enum_name, *tag, *field_index);
                    cur = self.emit_rvalue_to_temp(
                        payload_ty.clone(),
                        Rvalue::EnumPayload {
                            enum_name: enum_name.clone(),
                            tag: *tag,
                            operand: cur,
                            field_index: *field_index,
                        },
                        sp,
                    );
                    cur_ty = payload_ty;
                }
            }
        }
        cur
    }

    fn air_struct_field_type(&self, ty: &AirType, field: &str) -> AirType {
        if let AirType::Struct(name) = ty {
            if let Some(def) = self.structs.iter().find(|s| &s.name == name) {
                if let Some(f) = def.fields.iter().find(|f| f.name == field) {
                    return f.ty.clone();
                }
            }
        }
        AirType::Ptr(Box::new(AirType::Void))
    }

    fn air_enum_payload_type(&self, enum_name: &str, tag: u32, field_index: u32) -> AirType {
        if let Some(def) = self.enums.iter().find(|e| e.name == enum_name) {
            if let Some(v) = def.variants.iter().find(|v| v.tag == tag) {
                if let Some(ty) = v.payload.get(field_index as usize) {
                    return ty.clone();
                }
            }
        }
        AirType::Ptr(Box::new(AirType::Void))
    }

    pub(super) fn emit_carrier_field_retains(
        &mut self,
        base: Operand,
        base_ty: &AirType,
        paths: &[crate::rc_paths::RcLeafPath],
        sp: Option<Span>,
    ) {
        for path in paths {
            let leaf = self.emit_load_rc_leaf(base.clone(), base_ty, path, sp);
            self.emit_rc_retain(leaf, sp);
        }
    }

    pub(super) fn emit_carrier_field_releases(
        &mut self,
        base: LocalId,
        base_ty: &AirType,
        paths: &[crate::rc_paths::RcLeafPath],
        sp: Option<Span>,
    ) {
        for path in paths {
            let leaf = self.emit_load_rc_leaf(Operand::Copy(base), base_ty, path, sp);
            self.emit_rc_release_operand(leaf, sp);
        }
    }

    pub(super) fn emit_rc_release_operand(&mut self, ptr: Operand, sp: Option<Span>) {
        self.emit(
            AirStmtKind::CallVoid {
                func: Callee::Named("__aelys_rc_release".to_string()),
                args: vec![ptr],
            },
            sp,
        );
    }

    pub(super) fn carrier_paths_for(
        &mut self,
        sema_ty: &InferType,
    ) -> Option<(AirType, Vec<crate::rc_paths::RcLeafPath>)> {
        if !matches!(sema_ty, InferType::Struct(_) | InferType::Enum(_, _)) {
            return None;
        }
        let air_ty = self.lower_type_from_infer(sema_ty);
        match crate::rc_paths::rc_field_paths(&air_ty, &self.structs, &self.enums) {
            crate::rc_paths::RcScan::None => None,
            crate::rc_paths::RcScan::Paths(paths) => Some((air_ty, paths)),
            // skipping is only sound while generic structs never reach codegen; once they
            // monomorphize, an Rc carrier will slip through here and leak or UAF
            crate::rc_paths::RcScan::Undecidable(_) => None,
            crate::rc_paths::RcScan::RejectedMultiVariant(why) => {
                self.report_error(format!(
                    "[rc-stage1] {why}"
                ));
                None
            }
        }
    }

    pub(super) fn finalize_function_body(&mut self) {
        // seal any pending block (for example loop exit blocks) or unsealed statements with implicit return
        if self.pending_block_id.is_some()
            || (self.current_stmts.is_empty() && self.current_blocks.is_empty())
            || !self.current_stmts.is_empty()
        {
            self.seal_block(AirTerminator::Return(None));
        }
    }

    pub(super) fn lower_stmt(&mut self, stmt: &TypedStmt) {
        let sp = Some(self.span(&stmt.span));
        match &stmt.kind {
            TypedStmtKind::Expression(expr) => {
                self.lower_expr_discard(expr);
            }
            TypedStmtKind::Let {
                name,
                mutable,
                initializer,
                var_type,
                ..
            } => {
                let ty = self.lower_type_from_infer(var_type);
                // Optimization: for array initializers, emit stores directly to the named local
                // instead of going through a temp + copy
                if matches!(ty, AirType::Array(_, _)) {
                    match &initializer.kind {
                        TypedExprKind::ArrayLiteral { elements, .. } => {
                            // Evaluate all elements before registering the name so that
                            // `let arr = [arr[0], 1, 2]` reads the *outer* arr, not itself.
                            let elem_ops: Vec<Operand> =
                                elements.iter().map(|e| self.lower_expr(e)).collect();
                            let local = self.alloc_named_local(name, ty, true, sp);
                            for (i, elem_op) in elem_ops.into_iter().enumerate() {
                                self.emit(
                                    AirStmtKind::Assign {
                                        place: Place::Index(
                                            local,
                                            Operand::Const(AirConst::IntLiteral(i as i64)),
                                        ),
                                        rvalue: Rvalue::Use(elem_op),
                                    },
                                    sp,
                                );
                            }
                            return;
                        }
                        TypedExprKind::ArraySized {
                            size, fill_value, ..
                        } => {
                            let n = match &size.kind {
                                TypedExprKind::Int(v) => *v as u64,
                                _ => {
                                    self.report_error(
                                        "unsupported non-constant array size: \
                                         ArraySized requires a constant integer size expression"
                                            .to_string(),
                                    );
                                    0
                                }
                            };
                            let elem_air_ty = match var_type {
                                InferType::Array(inner, _) => self.lower_type_from_infer(inner),
                                _ => AirType::I64,
                            };
                            self.check_stack_array_size(&elem_air_ty, n);
                            // Evaluate fill value before registering the name.
                            let fill_op = if let Some(fv) = fill_value {
                                self.lower_expr(fv)
                            } else {
                                let elem_ty = match var_type {
                                    InferType::Array(inner, _) => self.lower_type_from_infer(inner),
                                    _ => AirType::I64,
                                };
                                Operand::Const(AirConst::ZeroInit(elem_ty))
                            };
                            let local = self.alloc_named_local(name, ty, true, sp);
                            for i in 0..n {
                                self.emit(
                                    AirStmtKind::Assign {
                                        place: Place::Index(
                                            local,
                                            Operand::Const(AirConst::IntLiteral(i as i64)),
                                        ),
                                        rvalue: Rvalue::Use(fill_op.clone()),
                                    },
                                    sp,
                                );
                            }
                            return;
                        }
                        _ => {}
                    }
                }
                // Evaluate the initializer before registering the name so that
                // `let x = x + 1` reads the *outer* x, not the new binding.
                let operand = self.lower_expr(initializer);

                let is_rc_binding = matches!(var_type, InferType::Rc(_));
                let is_clone = is_rc_binding
                    && matches!(
                        initializer.kind,
                        TypedExprKind::Identifier(_) | TypedExprKind::Member { .. }
                    );
                if is_clone {
                    self.emit_rc_retain(operand.clone(), sp);
                }

                let carrier_info = self.carrier_paths_for(var_type);
                let is_carrier_copy = carrier_info.is_some()
                    && matches!(
                        initializer.kind,
                        TypedExprKind::Identifier(_) | TypedExprKind::Member { .. }
                    );

                let local = self.alloc_named_local(name, ty, *mutable, sp);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(local),
                        rvalue: Rvalue::Use(operand),
                    },
                    sp,
                );

                if is_rc_binding {
                    let depth = self.locals_by_name.len();
                    self.rc_locals.push((local, depth));
                }
                if let Some((carrier_ty, paths)) = carrier_info {
                    if is_carrier_copy {
                        self.emit_carrier_field_retains(
                            Operand::Copy(local),
                            &carrier_ty,
                            &paths,
                            sp,
                        );
                    }
                    let depth = self.locals_by_name.len();
                    self.carrier_locals.push(crate::lower::CarrierLocal {
                        local,
                        ty: carrier_ty,
                        paths,
                        depth,
                    });
                }

                if matches!(var_type, InferType::Vec(_)) {
                    let is_vec_copy = matches!(
                        initializer.kind,
                        TypedExprKind::Identifier(_) | TypedExprKind::Member { .. }
                    );
                    if is_vec_copy {
                        self.emit_cow_retain(local, sp);
                    }
                    let depth = self.locals_by_name.len();
                    self.cow_locals.push((local, depth));
                }
            }
            TypedStmtKind::Block(stmts) => {
                self.lower_body(stmts);
            }
            TypedStmtKind::If {
                condition,
                then_branch,
                else_branch,
            } => {
                self.lower_if(condition, then_branch, else_branch.as_deref(), sp);
            }
            TypedStmtKind::While { condition, body } => {
                self.lower_while(condition, body, sp);
            }
            TypedStmtKind::For {
                iterator,
                start,
                end,
                inclusive,
                step,
                body,
            } => {
                self.lower_for(iterator, start, end, *inclusive, step.as_ref(), body);
            }
            TypedStmtKind::ForEach {
                iterator,
                iterable,
                elem_type,
                body,
            } => {
                self.lower_foreach(iterator, iterable, elem_type, body, sp);
            }
            TypedStmtKind::Return(val) => {
                if let Some(e) = val {
                    let ret_ty = self.lower_type_from_infer(&e.ty);
                    // opaque means the return type is unresolved Dynamic (e.g. an implicit return of a print/println call).
                    // lower the expression for side effects only and emit a void return.
                    if matches!(ret_ty, AirType::Opaque) {
                        self.lower_expr_discard(e);
                        self.emit_rc_releases_for_return(None);
                        self.seal_block(AirTerminator::Return(None));
                        return;
                    }
                }
                let operand = val.as_ref().map(|e| self.lower_expr(e));
                self.emit_rc_releases_for_return(operand.as_ref());
                self.seal_block(AirTerminator::Return(operand));
            }
            TypedStmtKind::Break => {
                if let Some(loop_ctx) = self.loop_stack.last() {
                    let exit = loop_ctx.exit;
                    if self.rc_live_below(loop_ctx.body_scope_depth) {
                        self.report_error(
                            "[rc] an Rc<T> live in a loop body is abandoned by `break`; \
                             non-local jumps out of an Rc's scope are not supported yet"
                                .to_string(),
                        );
                        self.seal_block(AirTerminator::Unreachable);
                    } else {
                        self.seal_block(AirTerminator::Goto(exit));
                    }
                } else {
                    // break outside loop: sema should have rejected this, but
                    // seal the block to prevent malformed AIR during error recovery.
                    self.report_error("break statement outside of loop".to_string());
                    self.seal_block(AirTerminator::Unreachable);
                }
            }
            TypedStmtKind::Continue => {
                if let Some(loop_ctx) = self.loop_stack.last() {
                    let header = loop_ctx.header;
                    if self.rc_live_below(loop_ctx.body_scope_depth) {
                        self.report_error(
                            "[rc] an Rc<T> live in a loop body is abandoned by `continue`; \
                             non-local jumps out of an Rc's scope are not supported yet"
                                .to_string(),
                        );
                        self.seal_block(AirTerminator::Unreachable);
                    } else {
                        self.seal_block(AirTerminator::Goto(header));
                    }
                } else {
                    self.report_error("continue statement outside of loop".to_string());
                    self.seal_block(AirTerminator::Unreachable);
                }
            }
            TypedStmtKind::Function(func) => {
                self.lower_function(func);
            }
            TypedStmtKind::Needs(_)
            | TypedStmtKind::StructDecl { .. }
            | TypedStmtKind::EnumDecl { .. } => {}
        }
    }

    // control flow desugaring
    pub(super) fn lower_if(
        &mut self,
        condition: &aelys_sema::TypedExpr,
        then_branch: &TypedStmt,
        else_branch: Option<&TypedStmt>,
        _sp: Option<Span>,
    ) {
        let cond = self.lower_expr(condition);
        let then_id = self.alloc_block_id();
        let else_id = self.alloc_block_id();
        let merge_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Branch {
            cond,
            then_block: then_id,
            else_block: if else_branch.is_some() {
                else_id
            } else {
                merge_id
            },
        });

        self.fixup_block_id_noop(then_id);
        self.lower_stmt(then_branch);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(merge_id));
        }

        if let Some(else_br) = else_branch {
            self.fixup_block_id_noop(else_id);
            self.lower_stmt(else_br);
            if !self.last_block_is_terminated() {
                self.seal_block(AirTerminator::Goto(merge_id));
            }
        }

        self.fixup_block_id_noop(merge_id);
    }

    pub(super) fn lower_while(
        &mut self,
        condition: &aelys_sema::TypedExpr,
        body: &TypedStmt,
        _sp: Option<Span>,
    ) {
        let header_id = self.alloc_block_id();
        let body_id = self.alloc_block_id();
        let exit_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Goto(header_id));

        self.fixup_block_id_noop(header_id);
        let cond = self.lower_expr(condition);
        self.seal_block(AirTerminator::Branch {
            cond,
            then_block: body_id,
            else_block: exit_id,
        });

        self.loop_stack.push(super::LoopBlocks {
            header: header_id,
            exit: exit_id,
            body_scope_depth: self.locals_by_name.len(),
        });
        self.fixup_block_id_noop(body_id);
        self.lower_stmt(body);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(header_id));
        }
        self.loop_stack.pop();

        self.fixup_block_id_noop(exit_id);
    }

    /// the old fixup_block_id(X) renamed the last sealed block to X
    /// With nested control flow that creates multiple blocks, the last block is
    /// some inner merge, not the branch entry. This nuked entire loop bodies.
    /// Now we set the pending id *before* lowering so the first seal_block picks it up.
    pub(super) fn fixup_block_id_noop(&mut self, target: BlockId) {
        if let Some(old) = self.pending_block_id {
            if old != target {
                self.block_aliases.push((old.0, target.0));
            }
        }
        self.pending_block_id = Some(target);
    }

    pub(super) fn resolve_block_aliases(&mut self) {
        if self.block_aliases.is_empty() {
            return;
        }
        let resolve = |id: &mut BlockId, aliases: &[(u32, u32)]| {
            let mut current = id.0;
            for _ in 0..aliases.len() {
                if let Some(&(_, to)) = aliases.iter().find(|(from, _)| *from == current) {
                    current = to;
                } else {
                    break;
                }
            }
            *id = BlockId(current);
        };
        for block in &mut self.current_blocks {
            let aliases = &self.block_aliases;
            match &mut block.terminator {
                AirTerminator::Goto(id) => resolve(id, aliases),
                AirTerminator::Branch {
                    then_block,
                    else_block,
                    ..
                } => {
                    resolve(then_block, aliases);
                    resolve(else_block, aliases);
                }
                AirTerminator::Switch {
                    targets, default, ..
                } => {
                    for (_, id) in targets {
                        resolve(id, aliases);
                    }
                    resolve(default, aliases);
                }
                AirTerminator::Invoke { normal, unwind, .. } => {
                    resolve(normal, aliases);
                    resolve(unwind, aliases);
                }
                AirTerminator::Return(_)
                | AirTerminator::Unreachable
                | AirTerminator::Unwind
                | AirTerminator::Panic { .. } => {}
            }
        }
        self.block_aliases.clear();
    }

    pub(super) fn last_block_is_terminated(&self) -> bool {
        if self.pending_block_id.is_some() {
            return false;
        }
        // a block is terminated if it has a terminator than Goto
        // Goto is a fallthrough to a merge block, not a definitive exit
        // other terminator return, unreachable, branch etc are definitive exits.
        self.current_stmts.is_empty()
            && self
                .current_blocks
                .last()
                .is_some_and(|b| !matches!(b.terminator, AirTerminator::Goto(_)))
    }
}

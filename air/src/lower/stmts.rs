use super::{LoweringContext, StrInit};
use crate::*;
use aelys_sema::{InferType, TypedExpr, TypedExprKind, TypedStmt, TypedStmtKind};

impl<'a> LoweringContext<'a> {
    pub(super) fn lower_body(&mut self, stmts: &[TypedStmt], scope_span: aelys_syntax::Span) {
        // save the current scope depth so inner `let` bindings don't leak out.
        let scope_depth = self.locals_by_name.len();
        for stmt in stmts {
            self.lower_stmt(stmt);
        }
        self.emit_scope_rc_releases(scope_depth);
        self.emit_scope_affine_drops(scope_depth, scope_span);
        self.locals_by_name.truncate(scope_depth);
    }

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
        let addr = self.addr_of_own_temp(local, &vec_ty, sp);
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

    fn emit_str_slot_call(&mut self, retain: bool, local: LocalId, sp: Option<Span>) {
        let ty = self.counted_local_type(local);
        let addr = self.addr_of_own_temp(local, &ty, sp);
        self.emit(
            AirStmtKind::CallVoid {
                func: Callee::Named(crate::counts::count_callee(&ty, retain).to_string()),
                args: vec![addr],
            },
            sp,
        );
    }

    // the slot keeps its own type: the resolution after mono decides what the count becomes
    fn counted_local_type(&self, local: LocalId) -> AirType {
        self.local_air_type(local).unwrap_or(AirType::Str)
    }

    pub(super) fn counted_operand_type(&self, val: &Operand) -> AirType {
        match val {
            Operand::Copy(id) | Operand::Move(id) => self.counted_local_type(*id),
            Operand::Const(_) => AirType::Str,
        }
    }

    pub(super) fn emit_str_retain(&mut self, local: LocalId, sp: Option<Span>) {
        self.emit_str_slot_call(true, local, sp);
    }

    pub(super) fn emit_str_release(&mut self, local: LocalId, sp: Option<Span>) {
        self.emit_str_slot_call(false, local, sp);
    }

    pub(super) fn emit_posed_release(&mut self, local: LocalId, sp: Option<Span>) {
        if matches!(self.local_air_type(local), Some(AirType::Vec(_))) {
            self.emit_cow_release(local, sp);
        } else {
            self.emit_str_release(local, sp);
        }
    }

    pub(super) fn pose_vec_call_result(&mut self, result: &Operand) {
        if let Operand::Copy(id) | Operand::Move(id) = result
            && matches!(self.local_air_type(*id), Some(AirType::Vec(_)))
            && !self.last_block_is_terminated()
        {
            self.stmt_str_temps.push((*id, self.current_blocks.len()));
        }
    }

    pub(super) fn claim_str_init(&mut self, val: &mut Operand) -> StrInit {
        if let Operand::Copy(id) | Operand::Move(id) = *val
            && let Some(at) = self.stmt_str_temps.iter().position(|(l, _)| *l == id)
        {
            self.stmt_str_temps.remove(at);
            *val = Operand::Move(id);
            return StrInit::Own;
        }
        self.str_init_kind(val)
    }

    pub(super) fn retained_str_copy(&mut self, val: Operand, sp: Option<Span>) -> Operand {
        let copy = self.alloc_temp(self.counted_operand_type(&val));
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(copy),
                rvalue: Rvalue::Use(val),
            },
            sp,
        );
        self.emit_str_retain(copy, sp);
        Operand::Copy(copy)
    }

    pub(super) fn own_for_store(&mut self, mut val: Operand, sp: Option<Span>) -> Operand {
        if matches!(self.claim_str_init(&mut val), StrInit::Own) {
            return val;
        }
        self.retained_str_copy(val, sp)
    }

    pub(super) fn own_str_for_store(
        &mut self,
        val: Operand,
        ty: &InferType,
        sp: Option<Span>,
    ) -> Operand {
        if self.counted(ty) {
            self.own_for_store(val, sp)
        } else {
            val
        }
    }

    pub(super) fn claim_owned_str_temp(&mut self, val: &Operand) -> bool {
        let (Operand::Copy(id) | Operand::Move(id)) = val else {
            return false;
        };
        let Some(at) = self.stmt_str_temps.iter().position(|(l, _)| l == id) else {
            return false;
        };
        self.stmt_str_temps.remove(at);
        true
    }

    pub(super) fn str_temps_posed_here(&self) -> Vec<LocalId> {
        let now = self.current_blocks.len();
        self.stmt_str_temps
            .iter()
            .filter(|(_, sealed)| *sealed == now)
            .map(|(id, _)| *id)
            .collect()
    }

    // capture right before sealing the branch, or the merge is not dominated by what was posed
    pub(super) fn restamp_str_temps(&mut self, dominated: &[LocalId]) {
        let now = self.current_blocks.len();
        for (id, sealed) in self.stmt_str_temps.iter_mut() {
            if dominated.contains(id) {
                *sealed = now;
            }
        }
    }

    pub(super) fn str_temp_ids(&self) -> Vec<LocalId> {
        self.stmt_str_temps.iter().map(|(id, _)| *id).collect()
    }

    pub(super) fn release_arm_str_temps(&mut self, before: &[LocalId]) {
        let now = self.current_blocks.len();
        let reachable = !self.last_block_is_terminated();
        let arm: Vec<(LocalId, usize)> = self
            .stmt_str_temps
            .iter()
            .filter(|(id, _)| !before.contains(id))
            .copied()
            .collect();
        for (id, sealed) in &arm {
            if reachable && *sealed == now {
                self.emit_posed_release(*id, None);
            }
        }
        self.stmt_str_temps.retain(|(id, _)| before.contains(id));
    }

    fn release_posed_str_temps(&mut self, keep: &[LocalId]) {
        let posed: Vec<LocalId> = self
            .stmt_str_temps
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| !keep.contains(id))
            .collect();
        for id in posed {
            self.emit_posed_release(id, None);
        }
    }

    pub(super) fn str_init_kind(&self, val: &Operand) -> StrInit {
        if matches!(val, Operand::Const(_)) || self.str_operand_is_fresh(val) {
            return StrInit::Own;
        }
        if let Operand::Copy(id) | Operand::Move(id) = val
            && self.str_locals.iter().any(|(local, _)| local == id)
        {
            return StrInit::Share;
        }
        StrInit::Borrow
    }

    pub(super) fn str_local_is_owned(&self, id: LocalId) -> bool {
        self.str_locals.iter().any(|(local, _)| *local == id)
    }

    pub(super) fn local_is_named(&self, id: LocalId) -> bool {
        self.current_locals
            .iter()
            .any(|l| l.id == id && l.name.is_some())
    }

    pub(super) fn str_binding_is_stable(&self, id: LocalId) -> bool {
        !self.capture_slots.contains_key(&id)
            && !self.stmt_str_temps.iter().any(|(l, _)| *l == id)
            && self
                .current_locals
                .iter()
                .any(|l| l.id == id && l.name.is_some() && !l.is_mut)
    }

    pub(super) fn forget_str_local(&mut self, id: LocalId) {
        self.str_locals.retain(|(local, _)| *local != id);
    }

    pub(super) fn str_operand_is_fresh(&self, val: &Operand) -> bool {
        let (Operand::Copy(id) | Operand::Move(id)) = val else {
            return false;
        };
        if self.fresh_str_results.contains(id) {
            return true;
        }
        let Some(last) = self.current_stmts.last() else {
            return false;
        };
        let AirStmtKind::Assign {
            place: Place::Local(def),
            rvalue,
        } = &last.kind
        else {
            return false;
        };
        if def != id
            || !self
                .local_air_type(*id)
                .is_some_and(|ty| self.counted_air(&ty))
        {
            return false;
        }
        match rvalue {
            Rvalue::Call {
                func: Callee::Named(name),
                ..
            } => {
                crate::symbols::STRING_PRODUCER_SYMBOLS.contains(&name.as_str())
                    || self.fresh_returns.contains(name)
            }
            Rvalue::Call {
                func: Callee::FnPtr(_),
                ..
            } => true,
            Rvalue::BinaryOp(BinOp::Add, _, _) => true,
            // a literal took a share for every leaf as it was built
            Rvalue::StructInit { .. } | Rvalue::EnumInit { .. } => true,
            _ => false,
        }
    }

    pub(super) fn emit_cow_release_through_ptr(&mut self, ptr: Operand, sp: Option<Span>) {
        self.emit(
            AirStmtKind::CallVoid {
                func: Callee::Named("__aelys_vec_release".to_string()),
                args: vec![ptr],
            },
            sp,
        );
    }

    // a slot whose type is vec<t> owns exactly one counted share of its buffer. a value
    pub(super) fn emit_vec_slot_acquire(
        &mut self,
        slot_is_vec: bool,
        rhs: Option<&TypedExprKind>,
        val: &Operand,
        sp: Option<Span>,
    ) {
        if !slot_is_vec {
            return;
        }
        if rhs.is_some_and(Self::vec_rhs_is_fresh) {
            return;
        }
        match val {
            Operand::Copy(id) | Operand::Move(id) => self.emit_cow_retain(*id, sp),
            Operand::Const(_) => self.report_ice(
                "[cow] a Vec slot is initialised from a non-local operand; the share cannot be \
                 accounted (this is a compiler bug, not a program error)"
                    .to_string(),
            ),
        }
    }

    // the one place a vec-producing form's freshness is decided. a fresh producer owns the +1 the
    fn vec_rhs_is_fresh(kind: &TypedExprKind) -> bool {
        match kind {
            TypedExprKind::EnumVariant {
                enum_name, variant, ..
            } => enum_name == "Vec" && variant == "new",
            TypedExprKind::VecLiteral { .. } => true,
            // a callee transfers its share out: emit_rc_releases_for_return excludes the escaped
            TypedExprKind::Call { .. } => true,
            _ => false,
        }
    }

    pub(super) fn emit_scope_rc_releases(&mut self, scope_depth: usize) {
        let has_rc = self.rc_locals.iter().any(|(_, d)| *d > scope_depth);
        let has_carrier = self.carrier_locals.iter().any(|c| c.depth > scope_depth);
        let has_cow = self.cow_locals.iter().any(|(_, d)| *d > scope_depth);
        let has_str = self.str_locals.iter().any(|(_, d)| *d > scope_depth);
        if !has_rc && !has_carrier && !has_cow && !has_str {
            return;
        }
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
        let strs_to_release: Vec<LocalId> = self
            .str_locals
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
            for id in &strs_to_release {
                self.emit_str_release(*id, None);
            }
        }
        self.rc_locals.retain(|(_, d)| *d <= scope_depth);
        self.carrier_locals.retain(|c| c.depth <= scope_depth);
        self.cow_locals.retain(|(_, d)| *d <= scope_depth);
        self.str_locals.retain(|(_, d)| *d <= scope_depth);
    }

    pub(super) fn emit_scope_affine_drops(
        &mut self,
        scope_depth: usize,
        scope_span: aelys_syntax::Span,
    ) {
        if !self.affine_locals.iter().any(|a| a.depth > scope_depth) {
            return;
        }
        let key = crate::bir::drop_key(&scope_span);
        if !self.last_block_is_terminated() {
            let to_drop = self.collect_affine_drops(key, |d| d > scope_depth);
            for (local, id_field) in to_drop.into_iter().rev() {
                self.emit_affine_drop(local, &id_field, None);
            }
        }
        self.affine_locals.retain(|a| a.depth <= scope_depth);
    }

    // or returned. unlike borrowed rc/carrier params, which are never released callee-side.
    pub(super) fn emit_affine_param_drops_on_fallthrough(&mut self, func_span: aelys_syntax::Span) {
        if !self.affine_locals.iter().any(|a| a.depth == 0) {
            return;
        }
        let key = crate::bir::drop_key(&func_span);
        if !self.last_block_is_terminated() {
            let to_drop = self.collect_affine_drops(key, |d| d == 0);
            for (local, id_field) in to_drop {
                self.emit_affine_drop(local, &id_field, None);
            }
        }
        self.affine_locals.retain(|a| a.depth != 0);
    }

    // local moved earlier on the path is absent. it must not copy emit_rc_releases_for_return's
    pub(super) fn emit_affine_drops_for_return(&mut self, return_span: aelys_syntax::Span) {
        let key = crate::bir::drop_key(&return_span);
        let to_drop = self.collect_affine_drops(key, |_| true);
        for (local, id_field) in to_drop.into_iter().rev() {
            self.emit_affine_drop(local, &id_field, None);
        }
    }

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
        let strs_to_release: Vec<LocalId> = self
            .str_locals
            .iter()
            .map(|(id, _)| *id)
            .filter(|id| Some(*id) != escaped)
            .collect();
        for id in strs_to_release {
            self.emit_str_release(id, None);
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

    pub(super) fn emit_param_str_releases_on_fallthrough(&mut self) {
        if !self.str_locals.iter().any(|(_, d)| *d == 0) {
            return;
        }
        if !self.last_block_is_terminated() {
            let to_release: Vec<LocalId> = self
                .str_locals
                .iter()
                .filter(|(_, d)| *d == 0)
                .map(|(id, _)| *id)
                .collect();
            for id in to_release {
                self.emit_str_release(id, None);
            }
        }
        self.str_locals.retain(|(_, d)| *d != 0);
    }

    pub(super) fn rc_live_below(&self, threshold: usize) -> bool {
        self.rc_locals.iter().any(|(_, d)| *d > threshold)
            || self.carrier_locals.iter().any(|c| c.depth > threshold)
    }

    // a local kept open past its block may never have run on this path, so it stays for the scope end
    fn emit_cow_releases_below(&mut self, threshold: usize) {
        let below: Vec<LocalId> = self
            .cow_locals
            .iter()
            .filter(|(id, d)| *d > threshold && !self.cow_zombies.contains(id))
            .map(|(id, _)| *id)
            .collect();
        for id in below {
            self.emit_cow_release(id, None);
        }
    }

    fn emit_str_releases_below(&mut self, threshold: usize) {
        let below: Vec<LocalId> = self
            .str_locals
            .iter()
            .filter(|(_, d)| *d > threshold)
            .map(|(id, _)| *id)
            .collect();
        for id in below {
            self.emit_str_release(id, None);
        }
    }

    pub(super) fn affine_live_below(&self, threshold: usize) -> bool {
        self.affine_locals.iter().any(|a| a.depth > threshold)
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
                    enum_ref,
                    tag,
                    field_index,
                } => {
                    let payload_ty =
                        self.air_enum_payload_type(&enum_ref.symbol(), *tag, *field_index);
                    cur = self.emit_rvalue_to_temp(
                        payload_ty.clone(),
                        Rvalue::EnumPayload {
                            enum_ref: enum_ref.clone(),
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

    pub(super) fn air_struct_field_type(&self, ty: &AirType, field: &str) -> AirType {
        if let AirType::Struct(name) = ty {
            let mine = self.structs.iter();
            if let Some(def) = mine
                .chain(self.imported.structs.iter())
                .find(|s| &s.name == name)
            {
                if let Some(f) = def.fields.iter().find(|f| f.name == field) {
                    return f.ty.clone();
                }
            }
        }
        AirType::Ptr(Box::new(AirType::Void))
    }

    fn air_enum_payload_type(&self, enum_name: &str, tag: u32, field_index: u32) -> AirType {
        let mine = self.enums.iter();
        if let Some(def) = mine
            .chain(self.imported.enums.iter())
            .find(|e| e.name == enum_name)
        {
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
            crate::rc_paths::RcScan::Undecidable(_) => None,
            crate::rc_paths::RcScan::RejectedMultiVariant(why) => {
                self.report_unsupported(format!("[rc-stage1] {why}"));
                None
            }
        }
    }

    pub(super) fn finalize_function_body(&mut self) {
        if self.pending_block_id.is_some()
            || (self.current_stmts.is_empty() && self.current_blocks.is_empty())
            || !self.current_stmts.is_empty()
        {
            self.seal_block(AirTerminator::Return(None));
        }
    }

    pub(super) fn lower_stmt(&mut self, stmt: &TypedStmt) {
        let mark = self.stmt_str_temps.len();
        self.lower_stmt_kind(stmt);
        let pending: Vec<(LocalId, usize)> = self.stmt_str_temps.drain(mark..).collect();
        if self.last_block_is_terminated() {
            return;
        }
        for (id, sealed) in pending {
            // a block sealed since the value was made may skip it on some path, leaving the slot unset
            if sealed == self.current_blocks.len() {
                self.emit_posed_release(id, None);
            }
        }
    }

    fn lower_stmt_kind(&mut self, stmt: &TypedStmt) {
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
                let air_ty = ty.clone();
                let declared_len = match &ty {
                    AirType::Array(_, n) => Some(*n),
                    _ => None,
                };
                if let Some(declared_len) = declared_len {
                    match &initializer.kind {
                        TypedExprKind::ArrayLiteral { elements, .. } => {
                            let refs: Vec<&TypedExpr> = elements.iter().collect();
                            let elem_ops = self.lower_pinned_operands_owning(&refs, true, sp);
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
                            if self.counted_air(&air_ty) {
                                let depth = self.locals_by_name.len();
                                self.str_locals.push((local, depth));
                            }
                            return;
                        }
                        TypedExprKind::ArraySized { fill_value, .. } => {
                            // expression lets the optimizer's folding decide the verdict
                            let n = declared_len;
                            let elem_air_ty = match var_type {
                                InferType::Array(inner, _) => self.lower_type_from_infer(inner),
                                _ => AirType::I64,
                            };
                            self.check_stack_array_size(&elem_air_ty, n);
                            let fill_op = if let Some(fv) = fill_value {
                                let op = self.lower_expr(fv);
                                if n > 0 {
                                    self.own_str_for_store(op, &fv.ty, sp)
                                } else {
                                    op
                                }
                            } else {
                                let elem_ty = match var_type {
                                    InferType::Array(inner, _) => self.lower_type_from_infer(inner),
                                    _ => AirType::I64,
                                };
                                Operand::Const(AirConst::ZeroInit(elem_ty))
                            };
                            let local = self.alloc_named_local(name, ty, true, sp);
                            for i in 0..n {
                                if i > 0 {
                                    self.retain_str_fill(&fill_op, sp);
                                }
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
                            if self.counted_air(&air_ty) {
                                let depth = self.locals_by_name.len();
                                self.str_locals.push((local, depth));
                            }
                            return;
                        }
                        _ => {}
                    }
                }
                let mut operand = self.lower_expr(initializer);
                let str_init = self
                    .counted(var_type)
                    .then(|| self.claim_str_init(&mut operand));
                let from_param = matches!(&operand, Operand::Copy(id) | Operand::Move(id)
                    if self.str_param_borrows.contains(id));

                let vec_claimed =
                    matches!(var_type, InferType::Vec(_)) && self.claim_owned_str_temp(&operand);
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
                    if !vec_claimed {
                        self.emit_vec_slot_acquire(
                            true,
                            Some(&initializer.kind),
                            &Operand::Copy(local),
                            sp,
                        );
                    }
                    let depth = self.locals_by_name.len();
                    self.cow_locals.push((local, depth));
                }

                match str_init {
                    None => {}
                    // a parameter outlives the call and an immutable binding cannot, so it borrows
                    Some(StrInit::Borrow) if from_param && !*mutable => {}
                    Some(kind) => {
                        if !matches!(kind, StrInit::Own) {
                            self.emit_str_retain(local, sp);
                        }
                        let depth = self.locals_by_name.len();
                        self.str_locals.push((local, depth));
                    }
                }

                if self.affine_category(var_type).is_affine() {
                    let depth = self.locals_by_name.len();
                    self.affine_locals.push(crate::lower::AffineLocal {
                        local,
                        depth,
                        id_field: "id".to_string(),
                        decl_key: crate::bir::drop_key(&stmt.span),
                    });
                }
            }
            TypedStmtKind::Block(stmts) => {
                self.lower_body(stmts, stmt.span);
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
                    // opaque means the return type is unresolved dynamic (e.g. an implicit return of a print/println call).
                    if matches!(ret_ty, AirType::Opaque) {
                        self.lower_expr_discard(e);
                        self.emit_rc_releases_for_return(None);
                        self.emit_affine_drops_for_return(stmt.span);
                        self.seal_block(AirTerminator::Return(None));
                        return;
                    }
                }
                let returns_str = val.as_ref().is_some_and(|e| self.counted(&e.ty));
                // the environment keeps its own share, so what the closure hands out takes one
                let returns_capture = val.as_ref().is_some_and(|e| {
                    matches!(&e.kind, TypedExprKind::Identifier(n)
                        if self.lookup_local(n).is_some_and(|id| self.capture_slots.contains_key(&id)))
                });
                let operand = match val.as_ref().map(|e| self.lower_expr(e)) {
                    Some(Operand::Copy(id)) if self.str_local_is_owned(id) => {
                        Some(Operand::Move(id))
                    }
                    Some(op @ (Operand::Copy(id) | Operand::Move(id)))
                        if self.claim_owned_str_temp(&op) =>
                    {
                        Some(Operand::Move(id))
                    }
                    // a named binding hands its own share over, or the rc leaves it shares are released under it
                    Some(Operand::Copy(id) | Operand::Move(id))
                        if returns_str
                            && !self.str_operand_is_fresh(&Operand::Copy(id))
                            && !self.position_is_dead()
                            && self.local_is_named(id) =>
                    {
                        self.emit_str_retain(id, sp);
                        Some(Operand::Move(id))
                    }
                    Some(op @ (Operand::Copy(_) | Operand::Move(_)))
                        if returns_str
                            && !self.str_operand_is_fresh(&op)
                            && !self.position_is_dead() =>
                    {
                        let r = self.alloc_temp(self.counted_operand_type(&op));
                        self.emit(
                            AirStmtKind::Assign {
                                place: Place::Local(r),
                                rvalue: Rvalue::Use(op),
                            },
                            sp,
                        );
                        self.emit_str_retain(r, sp);
                        Some(Operand::Move(r))
                    }
                    other => other,
                };
                if returns_capture
                    && val
                        .as_ref()
                        .is_some_and(|e| matches!(e.ty, InferType::Vec(_)))
                    && let Some(op) = operand.as_ref()
                {
                    self.emit_vec_slot_acquire(true, None, op, sp);
                }
                if self.position_is_dead() {
                    self.seal_block(AirTerminator::Unreachable);
                    return;
                }
                self.release_posed_str_temps(&[]);
                self.emit_rc_releases_for_return(operand.as_ref());
                self.emit_affine_drops_for_return(stmt.span);
                self.seal_block(AirTerminator::Return(operand));
            }
            TypedStmtKind::Break => {
                if let Some(loop_ctx) = self.loop_stack.last() {
                    let exit = loop_ctx.exit;
                    let body_depth = loop_ctx.body_scope_depth;
                    let at_entry = loop_ctx.str_temps_at_entry.clone();
                    if self.rc_live_below(body_depth) {
                        self.report_unsupported(
                            "[rc] an Rc<T> live in a loop body is abandoned by `break`; \
                             non-local jumps out of an Rc's scope are not supported yet"
                                .to_string(),
                        );
                        self.seal_block(AirTerminator::Unreachable);
                    } else if self.affine_live_below(body_depth) {
                        self.report_unsupported(
                            "[move] an affine value live in a loop body is abandoned by `break`; \
                             non-local jumps out of an affine scope are not supported yet"
                                .to_string(),
                        );
                        self.seal_block(AirTerminator::Unreachable);
                    } else {
                        self.emit_str_releases_below(body_depth);
                        self.emit_cow_releases_below(body_depth);
                        self.release_posed_str_temps(&at_entry);
                        self.seal_block(AirTerminator::Goto(exit));
                    }
                } else {
                    // break outside loop: sema should have rejected this, but
                    self.report_program("break statement outside of loop".to_string());
                    self.seal_block(AirTerminator::Unreachable);
                }
            }
            TypedStmtKind::Continue => {
                if let Some(loop_ctx) = self.loop_stack.last() {
                    let header = loop_ctx.header;
                    let body_depth = loop_ctx.body_scope_depth;
                    let at_entry = loop_ctx.str_temps_at_entry.clone();
                    if self.rc_live_below(body_depth) {
                        self.report_unsupported(
                            "[rc] an Rc<T> live in a loop body is abandoned by `continue`; \
                             non-local jumps out of an Rc's scope are not supported yet"
                                .to_string(),
                        );
                        self.seal_block(AirTerminator::Unreachable);
                    } else if self.affine_live_below(body_depth) {
                        self.report_unsupported(
                            "[move] an affine value live in a loop body is abandoned by `continue`; \
                             non-local jumps out of an affine scope are not supported yet"
                                .to_string(),
                        );
                        self.seal_block(AirTerminator::Unreachable);
                    } else {
                        self.emit_str_releases_below(body_depth);
                        self.emit_cow_releases_below(body_depth);
                        self.release_posed_str_temps(&at_entry);
                        self.seal_block(AirTerminator::Goto(header));
                    }
                } else {
                    self.report_program("continue statement outside of loop".to_string());
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

    pub(super) fn lower_if(
        &mut self,
        condition: &aelys_sema::TypedExpr,
        then_branch: &TypedStmt,
        else_branch: Option<&TypedStmt>,
        _sp: Option<Span>,
    ) {
        let before = self.str_temp_ids();
        let cond = self.lower_expr(condition);
        // seal_block never passes through emit, so a dead condition would still build its branch
        if self.position_is_dead() {
            return;
        }
        let then_id = self.alloc_block_id();
        let else_id = self.alloc_block_id();
        let merge_id = self.alloc_block_id();

        self.release_arm_str_temps(&before);
        let dominated = self.str_temps_posed_here();
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
        self.restamp_str_temps(&dominated);
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

        let dominated = self.str_temps_posed_here();
        self.seal_block(AirTerminator::Goto(header_id));

        self.fixup_block_id_noop(header_id);
        let before = self.str_temp_ids();
        let cond = self.lower_expr(condition);
        if self.position_is_dead() {
            return;
        }
        self.release_arm_str_temps(&before);
        self.seal_block(AirTerminator::Branch {
            cond,
            then_block: body_id,
            else_block: exit_id,
        });

        self.loop_stack.push(super::LoopBlocks {
            header: header_id,
            exit: exit_id,
            body_scope_depth: self.locals_by_name.len(),
            str_temps_at_entry: self.str_temp_ids(),
        });
        self.fixup_block_id_noop(body_id);
        self.lower_stmt(body);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(header_id));
        }
        self.loop_stack.pop();

        self.fixup_block_id_noop(exit_id);
        self.restamp_str_temps(&dominated);
    }

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

    pub(super) fn position_is_dead(&self) -> bool {
        self.pending_block_id.is_none()
            && self
                .current_blocks
                .last()
                .is_some_and(|b| !matches!(b.terminator, AirTerminator::Goto(_)))
    }

    pub(super) fn last_block_is_terminated(&self) -> bool {
        if self.pending_block_id.is_some() {
            return false;
        }
        self.current_stmts.is_empty()
            && self
                .current_blocks
                .last()
                .is_some_and(|b| !matches!(b.terminator, AirTerminator::Goto(_)))
    }
}

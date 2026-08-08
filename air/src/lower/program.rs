use super::LoweringContext;
use crate::*;
use aelys_sema::{InferType, TypedFunction, TypedParam, TypedStmtKind};

impl<'a> LoweringContext<'a> {
    pub(super) fn lower_program(&mut self) {
        for stmt in &self.program.stmts {
            if let TypedStmtKind::StructDecl {
                name,
                type_params,
                fields,
            } = &stmt.kind
            {
                if type_params.is_empty() {
                    self.lower_struct_decl(name, type_params, fields, &stmt.span);
                }
            }
        }

        // Collect enum definitions
        for stmt in &self.program.stmts {
            if let TypedStmtKind::EnumDecl {
                name,
                type_params,
                variants,
            } = &stmt.kind
            {
                self.lower_enum_decl(name, type_params, variants, &stmt.span);
            }
        }

        let stmts: Vec<_> = self.program.stmts.clone();
        for stmt in &stmts {
            match &stmt.kind {
                TypedStmtKind::StructDecl { .. }
                | TypedStmtKind::EnumDecl { .. }
                | TypedStmtKind::Function(_) => {}
                _ => self.lower_toplevel_stmt(stmt),
            }
        }

        for stmt in &stmts {
            if let TypedStmtKind::Function(func) = &stmt.kind {
                self.lower_function(func);
            }
        }
    }

    fn lower_struct_decl(
        &mut self,
        name: &str,
        type_params: &[String],
        fields: &[(String, InferType)],
        span: &aelys_syntax::Span,
    ) {
        let air_type_params = self.lower_type_params(type_params);
        let air_fields = fields
            .iter()
            .map(|(fname, fty)| AirStructField {
                name: fname.clone(),
                ty: self.lower_type_from_infer(fty),
                offset: None,
            })
            .collect();
        self.structs.push(AirStructDef {
            name: name.to_string(),
            type_params: air_type_params,
            fields: air_fields,
            is_closure_env: false,
            span: Some(self.span(span)),
        });
        self.type_params_map.clear();
    }

    fn lower_enum_decl(
        &mut self,
        name: &str,
        type_params: &[String],
        variants: &[(String, u32, Vec<InferType>)],
        span: &aelys_syntax::Span,
    ) {
        let air_type_params = self.lower_type_params(type_params);
        let air_variants = variants
            .iter()
            .map(|(vname, vtag, data)| AirEnumVariant {
                name: vname.clone(),
                tag: *vtag,
                payload: data
                    .iter()
                    .map(|ty| self.lower_type_from_infer(ty))
                    .collect(),
            })
            .collect();
        self.enums.push(AirEnumDef {
            name: name.to_string(),
            type_params: air_type_params,
            variants: air_variants,
            span: Some(self.span(span)),
        });
        self.type_params_map.clear();
    }

    pub(super) fn lower_function(&mut self, func: &TypedFunction) {
        let saved_locals = std::mem::take(&mut self.current_locals);
        let saved_params = std::mem::take(&mut self.current_params);
        let saved_blocks = std::mem::take(&mut self.current_blocks);
        let saved_stmts = std::mem::take(&mut self.current_stmts);
        let saved_names = std::mem::take(&mut self.locals_by_name);
        let saved_rc_locals = std::mem::take(&mut self.rc_locals);
        let saved_cow_locals = std::mem::take(&mut self.cow_locals);
// next_local_id resets to 0 below, so a stale outer capture_slots would false-positive on
        let saved_capture_slots = std::mem::take(&mut self.capture_slots);
        let saved_affine_locals = std::mem::take(&mut self.affine_locals);
        let saved_aliases = std::mem::take(&mut self.block_aliases);
        let saved_pending = self.pending_block_id.take();
        let saved_next_local = self.next_local_id;
        let saved_next_block = self.next_block_id;
        self.next_local_id = 0;
        self.next_block_id = 0;

        let func_id = self.alloc_function_id();
        let gc_mode = self.gc_mode_for_function(func);
        let captures = self.runtime_captures(&func.captures);

        if !captures.is_empty() {
            self.lower_closure(func, &captures, func_id, gc_mode);
        } else {
            self.lower_plain_function(func, func_id, gc_mode);
        }

        self.current_locals = saved_locals;
        self.current_params = saved_params;
        self.current_blocks = saved_blocks;
        self.current_stmts = saved_stmts;
        self.locals_by_name = saved_names;
        self.rc_locals = saved_rc_locals;
        self.cow_locals = saved_cow_locals;
        self.capture_slots = saved_capture_slots;
        self.affine_locals = saved_affine_locals;
        self.block_aliases = saved_aliases;
        self.pending_block_id = saved_pending;
        self.next_local_id = saved_next_local;
        self.next_block_id = saved_next_block;
    }

    fn lower_plain_function(&mut self, func: &TypedFunction, func_id: FunctionId, gc_mode: GcMode) {
        let type_params = self.lower_type_params(&func.type_params);
        let params = self.lower_params(&func.params);
        // retain each Vec param's buffer at entry, see emit_cow_param_entry_retains
        self.retain_vec_params(&func.params, &params);
        self.register_affine_params(&func.params, &params);
        let mut ret_ty = self.lower_type_from_infer(&func.return_type);
        if ret_ty == AirType::Opaque {
            self.report_error(format!(
                "function `{}` has unresolved return type (Opaque); \
                 treating as void — this indicates a type inference failure",
                func.name
            ));
            ret_ty = AirType::Void;
        }
        // sema uses Null for both the null literal and an implicit void return, and only
        // the literal should become Ptr(Void)
        if ret_ty == AirType::Ptr(Box::new(AirType::Void)) {
            ret_ty = AirType::Void;
        }

        self.lower_body(&func.body, func.span);
        // explicit returns already released these, this covers the fall-through exit
        self.emit_param_cow_releases_on_fallthrough();
        self.emit_affine_param_drops_on_fallthrough(func.span);
        self.finalize_function_body();
        self.resolve_block_aliases();

        let air_func = AirFunction {
            id: func_id,
            name: func.name.clone(),
            gc_mode,
            type_params,
            params,
            ret_ty,
            locals: std::mem::take(&mut self.current_locals),
            blocks: std::mem::take(&mut self.current_blocks),
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: self.func_attribs(func),
            span: Some(self.span(&func.span)),
        };
        self.functions.push(air_func);
        self.type_params_map.clear();
    }

    fn lower_closure(
        &mut self,
        func: &TypedFunction,
        captures: &[(String, InferType)],
        func_id: FunctionId,
        gc_mode: GcMode,
    ) {
        let type_params = self.lower_type_params(&func.type_params);

        let env_name = format!("__closure_env_{}", func.name);
        let env_fields: Vec<AirStructField> = captures
            .iter()
            .map(|(name, ty)| AirStructField {
                name: name.clone(),
                ty: self.lower_type_from_infer(ty),
                offset: None,
            })
            .collect();

        self.structs.push(AirStructDef {
            name: env_name.clone(),
            type_params: Vec::new(),
            fields: env_fields,
            is_closure_env: true,
            span: Some(self.span(&func.span)),
        });

        let env_param_id = self.alloc_local_id();
        let env_ty = AirType::Ptr(Box::new(AirType::Struct(env_name.clone())));
        self.current_params.push(AirParam {
            id: env_param_id,
            ty: env_ty.clone(),
            name: "__env".to_string(),
            span: Some(self.span(&func.span)),
        });

// write-backs are gone, so a `&mut <capture>` that escapes into a call still writes
        let mut slot_pairs: Vec<(LocalId, String)> = Vec::new();
        for (cap_name, cap_ty) in captures {
            let cap_air = self.lower_type_from_infer(cap_ty);
            let ptr_local = self.addr_of_env_field(env_param_id, cap_name, &cap_air);
            self.rename_local(ptr_local, cap_name);
            slot_pairs.push((ptr_local, cap_name.clone()));
        }

        let saved_env_param = self.closure_env_param.replace(env_param_id);
        let saved_capture_slots =
            std::mem::replace(&mut self.capture_slots, slot_pairs.into_iter().collect());

        let user_params = self.lower_params(&func.params);
        self.retain_vec_params(&func.params, &user_params);
        self.register_affine_params(&func.params, &user_params);
        let mut ret_ty = self.lower_type_from_infer(&func.return_type);
        if ret_ty == AirType::Opaque {
            self.report_error(format!(
                "closure `{}` has unresolved return type (Opaque); \
                 treating as void — this indicates a type inference failure",
                func.name
            ));
            ret_ty = AirType::Void;
        }
        if ret_ty == AirType::Ptr(Box::new(AirType::Void)) {
            ret_ty = AirType::Void;
        }

        self.lower_body(&func.body, func.span);
        self.emit_param_cow_releases_on_fallthrough();
        self.emit_affine_param_drops_on_fallthrough(func.span);
        self.finalize_function_body();
        self.resolve_block_aliases();

        // Restore outer closure context (supports nested closures)
        self.closure_env_param = saved_env_param;
        self.capture_slots = saved_capture_slots;

        let mut all_params = vec![self.current_params.remove(0)];
        all_params.extend(user_params);

        let air_func = AirFunction {
            id: func_id,
            name: func.name.clone(),
            gc_mode,
            type_params,
            params: all_params,
            ret_ty,
            locals: std::mem::take(&mut self.current_locals),
            blocks: std::mem::take(&mut self.current_blocks),
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: self.func_attribs(func),
            span: Some(self.span(&func.span)),
        };
        self.functions.push(air_func);
        self.type_params_map.clear();
    }

    // every lambda gets an __env param, capturing or not, so the calling convention at a
    // call site never depends on whether the callee captures
    pub(super) fn lower_function_as_closure(&mut self, func: &TypedFunction) {
        let saved_locals = std::mem::take(&mut self.current_locals);
        let saved_params = std::mem::take(&mut self.current_params);
        let saved_blocks = std::mem::take(&mut self.current_blocks);
        let saved_stmts = std::mem::take(&mut self.current_stmts);
        let saved_names = std::mem::take(&mut self.locals_by_name);
        let saved_rc_locals = std::mem::take(&mut self.rc_locals);
        let saved_cow_locals = std::mem::take(&mut self.cow_locals);
// next_local_id resets to 0 below, so a stale outer capture_slots would false-positive on
        let saved_capture_slots = std::mem::take(&mut self.capture_slots);
        let saved_affine_locals = std::mem::take(&mut self.affine_locals);
        let saved_aliases = std::mem::take(&mut self.block_aliases);
        let saved_pending = self.pending_block_id.take();
        let saved_next_local = self.next_local_id;
        let saved_next_block = self.next_block_id;
        self.next_local_id = 0;
        self.next_block_id = 0;

        let func_id = self.alloc_function_id();
        let gc_mode = self.gc_mode_for_function(func);
        let captures = self.runtime_captures(&func.captures);

        // Always take the closure path
        self.lower_closure(func, &captures, func_id, gc_mode);

        self.current_locals = saved_locals;
        self.current_params = saved_params;
        self.current_blocks = saved_blocks;
        self.current_stmts = saved_stmts;
        self.locals_by_name = saved_names;
        self.rc_locals = saved_rc_locals;
        self.cow_locals = saved_cow_locals;
        self.capture_slots = saved_capture_slots;
        self.affine_locals = saved_affine_locals;
        self.block_aliases = saved_aliases;
        self.pending_block_id = saved_pending;
        self.next_local_id = saved_next_local;
        self.next_block_id = saved_next_block;
    }

    pub(super) fn runtime_captures(&self, captures: &[(String, InferType)]) -> Vec<(String, InferType)> {
        // File-scope lets live in global storage, not in closure environments.
        captures
            .iter()
            .filter(|(name, _)| !self.globals.iter().any(|global| global.name == *name))
            .cloned()
            .collect()
    }

    pub(super) fn lower_params(&mut self, params: &[TypedParam]) -> Vec<AirParam> {
        params
            .iter()
            .map(|p| {
                let ty = self.lower_type_from_infer(&p.ty);
                let id = self.alloc_named_local(
                    &p.name,
                    ty.clone(),
                    p.mutable,
                    Some(self.span(&p.span)),
                );
                AirParam {
                    id,
                    ty,
                    name: p.name.clone(),
                    span: Some(self.span(&p.span)),
                }
            })
            .collect()
    }

    // a Vec param aliases the caller's buffer, so without this retain the callee would see
    // refcount 1, take the in-place push path and mutate the caller's Vec. unlike an Rc
    // param, which is borrowed and never touched, a Vec param has value semantics
    pub(super) fn retain_vec_params(&mut self, params: &[TypedParam], air_params: &[AirParam]) {
        // depth 0 keeps them function-level, so no inner scope releases them
        for (p, air) in params.iter().zip(air_params.iter()) {
            if matches!(p.ty, InferType::Vec(_)) {
                self.emit_cow_retain(air.id, Some(self.span(&p.span)));
                self.cow_locals.push((air.id, 0));
            }
        }
    }

    pub(super) fn register_affine_params(&mut self, params: &[TypedParam], air_params: &[AirParam]) {
        for (p, air) in params.iter().zip(air_params.iter()) {
            if matches!(self.affine_category(&p.ty), crate::bir::Category::Affine) {
                self.affine_locals.push(crate::lower::AffineLocal {
                    local: air.id,
                    depth: 0,
                    id_field: "id".to_string(),
                    decl_key: crate::bir::drop_key(&p.span),
                });
            }
        }
    }

    pub(super) fn func_attribs(&self, func: &TypedFunction) -> FunctionAttribs {
        let inline = if func.decorators.iter().any(|d| d.name == "inline_always") {
            InlineHint::Always
        } else if func.decorators.iter().any(|d| d.name == "inline_never") {
            InlineHint::Never
        } else {
            InlineHint::Default
        };
        FunctionAttribs {
            inline,
            no_gc: func.decorators.iter().any(|d| d.name == "no_gc"),
            no_unwind: false,
            cold: func.decorators.iter().any(|d| d.name == "cold"),
        }
    }

    fn lower_toplevel_stmt(&mut self, stmt: &aelys_sema::TypedStmt) {
        if let TypedStmtKind::Let {
            name,
            initializer,
            var_type,
            ..
        } = &stmt.kind
        {
            let ty = self.lower_type_from_infer(var_type);
            let init = self.try_global_const_expr(initializer);
            if let Some(message) = self.global_initializer_error(name, &ty, init.as_ref()) {
                self.report_error(message);
            }
            self.globals.push(AirGlobal {
                name: name.clone(),
                ty,
                init,
                gc_mode: self.file_gc_mode,
                span: Some(self.span(&stmt.span)),
            });
        }
    }

    // unlike try_const_expr this can emit, so it also folds non-capturing lambdas
    pub(super) fn try_global_const_expr(&mut self, expr: &aelys_sema::TypedExpr) -> Option<AirConst> {
        use aelys_sema::TypedExprKind;
        match &expr.kind {
            TypedExprKind::Lambda(inner) => self.try_global_const_expr(inner),
            TypedExprKind::LambdaInner { params, return_type, body, captures } => {
                let runtime_caps = self.runtime_captures(captures);
                if !runtime_caps.is_empty() {
                    return None; // capturing lambdas cannot be global constants
                }
                let lambda_name = format!("__lambda_{}", self.next_function_id);
                let fake_func = TypedFunction {
                    name: lambda_name.clone(),
                    type_params: Vec::new(),
                    params: params.clone(),
                    return_type: return_type.clone(),
                    body: body.clone(),
                    decorators: Vec::new(),
                    is_pub: false,
                    declared_nogc: false,
                    span: expr.span,
                    captures: Vec::new(),
                };
                self.lower_function_as_closure(&fake_func);
                Some(AirConst::FnRef(lambda_name))
            }
            TypedExprKind::ArrayLiteral { elements } => {
                let elements = elements.clone();
                let mut consts = Vec::with_capacity(elements.len());
                for e in &elements {
                    consts.push(self.try_global_const_expr(e)?);
                }
                Some(AirConst::Array(consts))
            }
            TypedExprKind::StructLiteral { name, fields } => {
                let name = name.clone();
                let fields = fields.clone();
                let mut field_consts = Vec::with_capacity(fields.len());
                for (fname, fexpr) in &fields {
                    field_consts.push((fname.clone(), self.try_global_const_expr(fexpr)?));
                }
                Some(AirConst::Struct { name, fields: field_consts })
            }
            _ => self.try_const_expr(expr),
        }
    }

    fn global_initializer_error(
        &self,
        name: &str,
        ty: &AirType,
        init: Option<&AirConst>,
    ) -> Option<String> {
        let Some(init) = init else {
            return Some(format!(
                "file-scope let '{name}' requires a compile-time constant initializer"
            ));
        };
        if matches!(ty, AirType::Enum(_)) && Self::enum_payload_needs_runtime_storage(init) {
            // globals are raw constant bytes, so a payload holding a runtime address must
            // fail here rather than drift into a backend-only error
            return Some(format!(
                "file-scope let '{name}' uses enum payload values with runtime-backed storage (`str`/`fnptr`), which globals cannot serialize yet"
            ));
        }
        None
    }

    fn enum_payload_needs_runtime_storage(init: &AirConst) -> bool {
        match init {
            AirConst::Str(_) | AirConst::FnRef(_) => true,
            AirConst::Enum { payload, .. } => payload
                .iter()
                .any(Self::enum_payload_needs_runtime_storage),
            _ => false,
        }
    }

    fn resolve_const_global_alias(&self, name: &str) -> Option<AirConst> {
        let mut current = name.to_string();
        let mut seen = std::collections::HashSet::new();

        loop {
            if !seen.insert(current.clone()) {
                return None;
            }

            let global = self.globals.iter().find(|global| global.name == current)?;
            let init = global.init.as_ref()?.clone();
            match init {
                // Follow fnptr aliases through prior globals until we reach the real symbol.
                AirConst::FnRef(target) if self.globals.iter().any(|global| global.name == target) => {
                    current = target;
                }
                other => return Some(other),
            }
        }
    }

    pub(super) fn try_const_expr(&self, expr: &aelys_sema::TypedExpr) -> Option<AirConst> {
        use aelys_sema::TypedExprKind;
        match &expr.kind {
            TypedExprKind::Int(v) => {
                if expr.ty.is_integer() {
                    Some(AirConst::Int(*v, super::infer_to_int_size(&expr.ty)))
                } else {
                    Some(AirConst::IntLiteral(*v))
                }
            }
            TypedExprKind::Float(v) => {
                let size = if matches!(expr.ty, InferType::F32) {
                    AirFloatSize::F32
                } else {
                    AirFloatSize::F64
                };
                Some(AirConst::Float(*v, size))
            }
            TypedExprKind::Bool(v) => Some(AirConst::Bool(*v)),
            TypedExprKind::String(v) => Some(AirConst::Str(v.clone())),
            TypedExprKind::Null => Some(AirConst::Null),
            TypedExprKind::Identifier(name) => {
                if let Some(existing) = self.resolve_const_global_alias(name) {
                    Some(existing)
                } else if matches!(expr.ty, InferType::Function { .. }) {
                    Some(AirConst::FnRef(name.clone()))
                } else {
                    None
                }
            }
            TypedExprKind::EnumVariant {
                enum_name,
                variant,
                tag,
                args,
            }
                if args.is_empty()
                    && self
                        .program
                        .type_table
                        .get_enum(enum_name)
                        .is_some_and(|def| def
                            .variants
                            .iter()
                            .any(|candidate| candidate.name == *variant && candidate.data.is_empty())) =>
            {
                Some(AirConst::Int(*tag as i64, AirIntSize::I32))
            }
            TypedExprKind::EnumVariant { tag, args, .. } => {
                let payload = args
                    .iter()
                    .map(|arg| self.try_const_expr(arg))
                    .collect::<Option<Vec<_>>>()?;
                // globals skip EnumInit, so carry the monomorphized name here
                let AirType::Enum(enum_name) = self.lower_type_from_infer(&expr.ty) else {
                    return None;
                };
                Some(AirConst::Enum {
                    enum_name,
                    tag: *tag,
                    payload,
                })
            }
            TypedExprKind::ArrayLiteral { elements } => {
                let consts: Option<Vec<AirConst>> =
                    elements.iter().map(|e| self.try_const_expr(e)).collect();
                consts.map(AirConst::Array)
            }
            TypedExprKind::ArraySized { size, fill_value } => {
                // [val; N] is constant if val is constant and N is a literal
                let n = if let TypedExprKind::Int(n) = &size.kind {
                    Some(*n as usize)
                } else {
                    None
                }?;
                let fill = fill_value.as_ref().and_then(|fv| self.try_const_expr(fv))?;
                Some(AirConst::Array(vec![fill; n]))
            }
            TypedExprKind::StructLiteral { name, fields } => {
                let field_consts: Option<Vec<(String, AirConst)>> = fields
                    .iter()
                    .map(|(fname, fexpr)| {
                        self.try_const_expr(fexpr).map(|c| (fname.clone(), c))
                    })
                    .collect();
                field_consts.map(|fields| AirConst::Struct { name: name.clone(), fields })
            }
            _ => None,
        }
    }
}


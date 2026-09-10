use super::LoweringContext;
use crate::*;
use aelys_common::Fault;
use aelys_sema::{InferType, TypedFunction, TypedParam, TypedStmtKind};
use aelys_syntax::ForeignConv;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ConstFoldFailure {
    NotAConstant,
    // the value is fixed at compile time and this compiler does not materialize it yet
    FoldNotImplemented,
    Invariant,
}

impl ConstFoldFailure {
    pub(super) fn fault(self) -> Fault {
        match self {
            ConstFoldFailure::NotAConstant => Fault::Program,
            ConstFoldFailure::FoldNotImplemented => Fault::Unsupported,
            ConstFoldFailure::Invariant => Fault::Compiler,
        }
    }

    fn rank(self) -> u8 {
        match self {
            ConstFoldFailure::FoldNotImplemented => 0,
            ConstFoldFailure::NotAConstant => 1,
            ConstFoldFailure::Invariant => 2,
        }
    }

    fn worse(self, other: ConstFoldFailure) -> ConstFoldFailure {
        if other.rank() > self.rank() { other } else { self }
    }
}

impl<'a> LoweringContext<'a> {
    pub(super) fn lower_program(&mut self) {
        let mut lowered_structs: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for stmt in &self.program.stmts {
            if let TypedStmtKind::StructDecl {
                name,
                type_params,
                fields,
                ..
            } = &stmt.kind
            {
                if type_params.is_empty() && lowered_structs.insert(name.clone()) {
                    self.lower_struct_decl(name, type_params, fields, &stmt.span);
                }
            }
        }

        let mut lowered_enums: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        for stmt in &self.program.stmts {
            if let TypedStmtKind::EnumDecl {
                name,
                type_params,
                variants,
                ..
            } = &stmt.kind
                && lowered_enums.insert(name.clone())
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

        if let Some(foreign) = &func.foreign {
            let conv = match foreign.calling_conv {
                ForeignConv::C => CallingConv::C,
            };
            self.lower_extern_function(func, func_id, gc_mode, conv);
        } else if !captures.is_empty() {
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

    fn lowered_return_type(&mut self, func: &TypedFunction, noun: &str) -> AirType {
        let mut ret_ty = self.lower_type_from_infer(&func.return_type);
        if ret_ty == AirType::Opaque {
            self.report_ice(format!(
                "{} `{}` has unresolved return type (Opaque); \
                 treating as void — this indicates a type inference failure",
                noun, func.name
            ));
            ret_ty = AirType::Void;
        }
        if ret_ty == AirType::Ptr(Box::new(AirType::Void)) {
            ret_ty = AirType::Void;
        }
        ret_ty
    }

    fn lower_extern_function(
        &mut self,
        func: &TypedFunction,
        func_id: FunctionId,
        gc_mode: GcMode,
        calling_conv: CallingConv,
    ) {
        let type_params = self.lower_type_params(&func.type_params);
        let params = self.lower_params(&func.params);
        let ret_ty = self.lowered_return_type(func, "function");

        let air_func = AirFunction {
            id: func_id,
            name: func.name.clone(),
            gc_mode,
            type_params,
            params,
            ret_ty,
            locals: Vec::new(),
            blocks: Vec::new(),
            is_extern: true,
            calling_conv,
            attributes: self.func_attribs(func),
            span: Some(self.span(&func.span)),
        };
        self.functions.push(air_func);
        self.current_locals.clear();
        self.type_params_map.clear();
    }

    fn lower_plain_function(&mut self, func: &TypedFunction, func_id: FunctionId, gc_mode: GcMode) {
        let type_params = self.lower_type_params(&func.type_params);
        let params = self.lower_params(&func.params);
        self.retain_vec_params(&func.params, &params);
        self.register_affine_params(&func.params, &params);
        let ret_ty = self.lowered_return_type(func, "function");

        self.lower_body(&func.body, func.span);
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
        let ret_ty = self.lowered_return_type(func, "closure");

        self.lower_body(&func.body, func.span);
        self.emit_param_cow_releases_on_fallthrough();
        self.emit_affine_param_drops_on_fallthrough(func.span);
        self.finalize_function_body();
        self.resolve_block_aliases();

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

    pub(super) fn runtime_captures(
        &self,
        captures: &[(String, InferType)],
    ) -> Vec<(String, InferType)> {
        captures
            .iter()
            .filter(|(name, _)| !self.is_global_name(name))
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

    // a vec param aliases the caller's buffer, so without this retain the callee would see
    pub(super) fn retain_vec_params(&mut self, params: &[TypedParam], air_params: &[AirParam]) {
        for (p, air) in params.iter().zip(air_params.iter()) {
            if matches!(p.ty, InferType::Vec(_)) {
                self.emit_cow_retain(air.id, Some(self.span(&p.span)));
                self.cow_locals.push((air.id, 0));
            }
        }
    }

    pub(super) fn register_affine_params(
        &mut self,
        params: &[TypedParam],
        air_params: &[AirParam],
    ) {
        for (p, air) in params.iter().zip(air_params.iter()) {
            if self.affine_category(&p.ty).is_affine() {
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
            let folded = self.try_global_const_expr(initializer);
            if let Some((fault, message)) = Self::global_initializer_error(name, &ty, &folded) {
                self.report(fault, message);
            }
            let init = folded.ok();
            self.globals.push(AirGlobal {
                name: name.clone(),
                ty,
                init,
                gc_mode: self.file_gc_mode,
                span: Some(self.span(&stmt.span)),
            });
        }
    }

    pub(super) fn try_global_const_expr(
        &mut self,
        expr: &aelys_sema::TypedExpr,
    ) -> Result<AirConst, ConstFoldFailure> {
        use aelys_sema::TypedExprKind;
        match &expr.kind {
            TypedExprKind::Lambda(inner) => self.try_global_const_expr(inner),
            TypedExprKind::LambdaInner {
                params,
                return_type,
                body,
                captures,
            } => {
                let runtime_caps = self.runtime_captures(captures);
                if !runtime_caps.is_empty() {
                    return Err(ConstFoldFailure::NotAConstant);
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
                    foreign: None,
                    span: expr.span,
                    captures: Vec::new(),
                };
                self.lower_function_as_closure(&fake_func);
                Ok(AirConst::FnRef(lambda_name))
            }
            TypedExprKind::ArrayLiteral { elements } => {
                let elements = elements.clone();
                let mut consts = Vec::with_capacity(elements.len());
                for e in &elements {
                    consts.push(self.try_global_const_expr(e)?);
                }
                Ok(AirConst::Array(consts))
            }
            TypedExprKind::StructLiteral { name, fields } => {
                let name = name.clone();
                let fields = fields.clone();
                let mut field_consts = Vec::with_capacity(fields.len());
                for (fname, fexpr) in &fields {
                    field_consts.push((fname.clone(), self.try_global_const_expr(fexpr)?));
                }
                Ok(AirConst::Struct {
                    name,
                    fields: field_consts,
                })
            }
            _ => self.try_const_expr(expr),
        }
    }

    fn global_initializer_error(
        name: &str,
        ty: &AirType,
        init: &Result<AirConst, ConstFoldFailure>,
    ) -> Option<(Fault, String)> {
        let init = match init {
            Ok(init) => init,
            Err(failure) => {
                return Some((
                    failure.fault(),
                    format!(
                        "file-scope let '{name}' requires a compile-time constant initializer"
                    ),
                ));
            }
        };
        if matches!(ty, AirType::Enum(_)) && Self::enum_payload_needs_runtime_storage(init) {
            return Some((
                Fault::Unsupported,
                format!(
                    "file-scope let '{name}' uses enum payload values with runtime-backed storage (`str`/`fnptr`), which globals cannot serialize yet"
                ),
            ));
        }
        None
    }

    fn enum_payload_needs_runtime_storage(init: &AirConst) -> bool {
        match init {
            AirConst::Str(_) | AirConst::FnRef(_) => true,
            AirConst::Enum { payload, .. } => {
                payload.iter().any(Self::enum_payload_needs_runtime_storage)
            }
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
                // follow fnptr aliases through prior globals until we reach the real symbol.
                AirConst::FnRef(target)
                    if self.globals.iter().any(|global| global.name == target) =>
                {
                    current = target;
                }
                other => return Some(other),
            }
        }
    }

    pub(super) fn try_const_expr(
        &self,
        expr: &aelys_sema::TypedExpr,
    ) -> Result<AirConst, ConstFoldFailure> {
        use aelys_sema::TypedExprKind;
        match &expr.kind {
            TypedExprKind::Int(v) => {
                if expr.ty.is_integer() {
                    Ok(AirConst::Int(*v, super::infer_to_int_size(&expr.ty)))
                } else {
                    Ok(AirConst::IntLiteral(*v))
                }
            }
            TypedExprKind::Float(v) => {
                let size = if matches!(expr.ty, InferType::F32) {
                    AirFloatSize::F32
                } else {
                    AirFloatSize::F64
                };
                Ok(AirConst::Float(*v, size))
            }
            TypedExprKind::Bool(v) => Ok(AirConst::Bool(*v)),
            TypedExprKind::String(v) => Ok(AirConst::Str(v.clone())),
            TypedExprKind::Null => Ok(AirConst::Null),
            TypedExprKind::Identifier(name) => {
                if let Some(existing) = self.resolve_const_global_alias(name) {
                    Ok(existing)
                } else if matches!(expr.ty, InferType::Function { .. }) {
                    Ok(AirConst::FnRef(name.clone()))
                } else {
                    Err(ConstFoldFailure::NotAConstant)
                }
            }
            TypedExprKind::EnumVariant {
                enum_name,
                variant,
                tag,
                args,
            } if args.is_empty()
                && self
                    .program
                    .type_table
                    .get_enum(enum_name)
                    .is_some_and(|def| {
                        def.variants.iter().any(|candidate| {
                            candidate.name == *variant && candidate.data.is_empty()
                        })
                    }) =>
            {
                Ok(AirConst::Int(*tag as i64, AirIntSize::I32))
            }
            TypedExprKind::EnumVariant { tag, args, .. } => {
                let payload = args
                    .iter()
                    .map(|arg| self.try_const_expr(arg))
                    .collect::<Result<Vec<_>, _>>()?;
                let AirType::Enum(enum_ref) = self.lower_type_from_infer(&expr.ty) else {
                    return Err(ConstFoldFailure::Invariant);
                };
                Ok(AirConst::Enum {
                    enum_ref,
                    tag: *tag,
                    payload,
                })
            }
            TypedExprKind::ArrayLiteral { elements } => {
                let consts: Result<Vec<AirConst>, _> =
                    elements.iter().map(|e| self.try_const_expr(e)).collect();
                consts.map(AirConst::Array)
            }
            TypedExprKind::ArraySized { size, fill_value } => {
                let TypedExprKind::Int(n) = &size.kind else {
                    return Err(self.fold_failure(size));
                };
                let n = *n as usize;
                let Some(fv) = fill_value.as_ref() else {
                    return Err(ConstFoldFailure::FoldNotImplemented);
                };
                let fill = self.try_const_expr(fv)?;
                Ok(AirConst::Array(vec![fill; n]))
            }
            TypedExprKind::StructLiteral { name, fields } => {
                let field_consts: Result<Vec<(String, AirConst)>, _> = fields
                    .iter()
                    .map(|(fname, fexpr)| self.try_const_expr(fexpr).map(|c| (fname.clone(), c)))
                    .collect();
                field_consts.map(|fields| AirConst::Struct {
                    name: name.clone(),
                    fields,
                })
            }
            other => Err(self.fold_failure_of(other)),
        }
    }

    fn fold_failure(&self, expr: &aelys_sema::TypedExpr) -> ConstFoldFailure {
        self.try_const_expr(expr)
            .err()
            .unwrap_or(ConstFoldFailure::FoldNotImplemented)
    }

    // no wildcard arm: a new expression form must state whether a constant for it can exist
    fn fold_failure_of(&self, kind: &aelys_sema::TypedExprKind) -> ConstFoldFailure {
        use aelys_sema::TypedExprKind as K;
        use ConstFoldFailure::{FoldNotImplemented, NotAConstant};
        match kind {
            K::Int(_)
            | K::Float(_)
            | K::Bool(_)
            | K::String(_)
            | K::Null
            | K::Identifier(_)
            | K::EnumVariant { .. }
            | K::ArrayLiteral { .. }
            | K::ArraySized { .. }
            | K::StructLiteral { .. } => FoldNotImplemented,
            K::Lambda(_) | K::LambdaInner { .. } => FoldNotImplemented,
            K::Grouping(inner) => self.fold_failure(inner),
            K::Cast { expr, .. } => self.fold_failure(expr),
            K::Unary { operand, .. } => self.fold_failure(operand),
            K::Binary { left, right, .. }
            | K::And { left, right }
            | K::Or { left, right } => self.fold_failure(left).worse(self.fold_failure(right)),
            K::If {
                condition,
                then_branch,
                else_branch,
            } => self
                .fold_failure(condition)
                .worse(self.fold_failure(then_branch))
                .worse(self.fold_failure(else_branch)),
            K::VecLiteral { elements, .. } => elements
                .iter()
                .map(|e| self.fold_failure(e))
                .fold(FoldNotImplemented, ConstFoldFailure::worse),
            K::FmtString(_)
            | K::Call { .. }
            | K::Assign { .. }
            | K::Member { .. }
            | K::Index { .. }
            | K::IndexAssign { .. }
            | K::FieldAssign { .. }
            | K::Range { .. }
            | K::Slice { .. }
            | K::Reference { .. }
            | K::Deref(_)
            | K::DerefAssign { .. }
            | K::Match { .. }
            | K::ResultAssert { .. }
            | K::Block { .. } => NotAConstant,
        }
    }
}

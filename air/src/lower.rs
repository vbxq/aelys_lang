use crate::*;
use aelys_sema::{
    InferType, TypedExpr, TypedExprKind, TypedFmtStringPart, TypedFunction, TypedParam,
    TypedProgram, TypedStmt, TypedStmtKind,
};
use aelys_syntax::BinaryOp;

pub fn lower(program: &TypedProgram) -> AirProgram {
    let mut cx = LoweringContext::new(program);
    cx.lower_program();
    cx.finish()
}

pub fn lower_with_gc_mode(program: &TypedProgram, file_gc_mode: GcMode) -> AirProgram {
    let mut cx = LoweringContext::new(program);
    cx.file_gc_mode = file_gc_mode;
    cx.lower_program();
    cx.finish()
}

struct LoweringContext<'a> {
    program: &'a TypedProgram,
    functions: Vec<AirFunction>,
    structs: Vec<AirStructDef>,
    globals: Vec<AirGlobal>,
    source_files: Vec<String>,
    next_function_id: u32,
    next_local_id: u32,
    next_block_id: u32,
    file_gc_mode: GcMode,
    current_blocks: Vec<AirBlock>,
    current_locals: Vec<AirLocal>,
    current_params: Vec<AirParam>,
    current_stmts: Vec<AirStmt>,
    locals_by_name: Vec<(String, LocalId)>,
    loop_stack: Vec<LoopBlocks>,
    type_params_map: Vec<(String, TypeParamId)>,
    pending_block_id: Option<BlockId>,
    block_aliases: Vec<(u32, u32)>,
}

struct LoopBlocks {
    header: BlockId,
    exit: BlockId,
}

impl<'a> LoweringContext<'a> {
    fn new(program: &'a TypedProgram) -> Self {
        Self {
            program,
            functions: Vec::new(),
            structs: Vec::new(),
            globals: Vec::new(),
            source_files: vec![program.source.name.clone()],
            next_function_id: 0,
            next_local_id: 0,
            next_block_id: 0,
            file_gc_mode: GcMode::Managed,
            current_blocks: Vec::new(),
            current_locals: Vec::new(),
            current_params: Vec::new(),
            current_stmts: Vec::new(),
            locals_by_name: Vec::new(),
            loop_stack: Vec::new(),
            type_params_map: Vec::new(),
            pending_block_id: None,
            block_aliases: Vec::new(),
        }
    }

    fn finish(self) -> AirProgram {
        AirProgram {
            functions: self.functions,
            structs: self.structs,
            globals: self.globals,
            source_files: self.source_files,
            mono_instances: Vec::new(),
        }
    }

    fn alloc_function_id(&mut self) -> FunctionId {
        let id = FunctionId(self.next_function_id);
        self.next_function_id += 1;
        id
    }

    fn alloc_local_id(&mut self) -> LocalId {
        let id = LocalId(self.next_local_id);
        self.next_local_id += 1;
        id
    }

    fn alloc_block_id(&mut self) -> BlockId {
        let id = BlockId(self.next_block_id);
        self.next_block_id += 1;
        id
    }

    fn alloc_temp(&mut self, ty: AirType) -> LocalId {
        let id = self.alloc_local_id();
        self.current_locals.push(AirLocal {
            id,
            ty,
            name: None,
            is_mut: false,
            span: None,
        });
        id
    }

    fn alloc_named_local(
        &mut self,
        name: &str,
        ty: AirType,
        is_mut: bool,
        span: Option<Span>,
    ) -> LocalId {
        let id = self.alloc_local_id();
        self.current_locals.push(AirLocal {
            id,
            ty,
            name: Some(name.to_string()),
            is_mut,
            span,
        });
        self.locals_by_name.push((name.to_string(), id));
        id
    }

    fn lookup_local(&self, name: &str) -> Option<LocalId> {
        self.locals_by_name
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
    }

    fn emit(&mut self, kind: AirStmtKind, span: Option<Span>) {
        self.current_stmts.push(AirStmt { kind, span });
    }

    fn seal_block(&mut self, terminator: AirTerminator) -> BlockId {
        let id = self
            .pending_block_id
            .take()
            .unwrap_or_else(|| self.alloc_block_id());
        self.current_blocks.push(AirBlock {
            id,
            stmts: std::mem::take(&mut self.current_stmts),
            terminator,
        });
        id
    }

    fn span(&self, s: &aelys_syntax::Span) -> Span {
        Span {
            file: 0,
            lo: s.start as u32,
            hi: s.end as u32,
        }
    }

    fn lower_type_params(&mut self, type_params: &[String]) -> Vec<TypeParamId> {
        type_params
            .iter()
            .enumerate()
            .map(|(i, name)| {
                let id = TypeParamId(i as u32);
                self.type_params_map.push((name.clone(), id));
                id
            })
            .collect()
    }

    fn lower_type_from_infer(&self, ty: &InferType) -> AirType {
        match ty {
            InferType::I8 => AirType::I8,
            InferType::I16 => AirType::I16,
            InferType::I32 => AirType::I32,
            InferType::I64 => AirType::I64,
            InferType::U8 => AirType::U8,
            InferType::U16 => AirType::U16,
            InferType::U32 => AirType::U32,
            InferType::U64 => AirType::U64,
            InferType::F32 => AirType::F32,
            InferType::F64 => AirType::F64,
            InferType::Bool => AirType::Bool,
            InferType::String => AirType::Str,
            InferType::Null => AirType::Void,
            InferType::Function { params, ret } => AirType::FnPtr {
                params: params
                    .iter()
                    .map(|p| self.lower_type_from_infer(p))
                    .collect(),
                ret: Box::new(self.lower_type_from_infer(ret)),
                conv: CallingConv::Aelys,
            },
            InferType::Array(inner) => AirType::Slice(Box::new(self.lower_type_from_infer(inner))),
            InferType::Vec(inner) => AirType::Slice(Box::new(self.lower_type_from_infer(inner))),
            InferType::Tuple(_) => AirType::Void,
            InferType::Range => AirType::Void,
            InferType::Struct(name) => {
                if let Some((_, id)) = self.type_params_map.iter().find(|(n, _)| n == name) {
                    AirType::Param(*id)
                } else {
                    AirType::Struct(name.clone())
                }
            }
            InferType::Var(_) | InferType::Dynamic => AirType::I64,
        }
    }

    fn gc_mode_for_function(&self, func: &TypedFunction) -> GcMode {
        if func.decorators.iter().any(|d| d.name == "no_gc") {
            GcMode::Manual
        } else {
            self.file_gc_mode
        }
    }

    // ========================================================================
    // Program
    // ========================================================================

    fn lower_program(&mut self) {
        for stmt in &self.program.stmts {
            if let TypedStmtKind::StructDecl {
                name,
                type_params,
                fields,
            } = &stmt.kind
            {
                self.lower_struct_decl(name, type_params, fields, &stmt.span);
            }
        }

        let stmts: Vec<_> = self.program.stmts.clone();
        for stmt in &stmts {
            match &stmt.kind {
                TypedStmtKind::Function(func) => self.lower_function(func),
                TypedStmtKind::StructDecl { .. } => {}
                _ => self.lower_toplevel_stmt(stmt),
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

    // ========================================================================
    // Functions
    // ========================================================================

    fn lower_function(&mut self, func: &TypedFunction) {
        let saved_locals = std::mem::take(&mut self.current_locals);
        let saved_params = std::mem::take(&mut self.current_params);
        let saved_blocks = std::mem::take(&mut self.current_blocks);
        let saved_stmts = std::mem::take(&mut self.current_stmts);
        let saved_names = std::mem::take(&mut self.locals_by_name);
        let saved_aliases = std::mem::take(&mut self.block_aliases);
        let saved_pending = self.pending_block_id.take();
        let saved_next_local = self.next_local_id;
        let saved_next_block = self.next_block_id;
        self.next_local_id = 0;
        self.next_block_id = 0;

        let func_id = self.alloc_function_id();
        let gc_mode = self.gc_mode_for_function(func);

        if !func.captures.is_empty() {
            self.lower_closure(func, func_id, gc_mode);
        } else {
            self.lower_plain_function(func, func_id, gc_mode);
        }

        self.current_locals = saved_locals;
        self.current_params = saved_params;
        self.current_blocks = saved_blocks;
        self.current_stmts = saved_stmts;
        self.locals_by_name = saved_names;
        self.block_aliases = saved_aliases;
        self.pending_block_id = saved_pending;
        self.next_local_id = saved_next_local;
        self.next_block_id = saved_next_block;
    }

    fn lower_plain_function(&mut self, func: &TypedFunction, func_id: FunctionId, gc_mode: GcMode) {
        let type_params = self.lower_type_params(&func.type_params);
        let params = self.lower_params(&func.params);
        let ret_ty = self.lower_type_from_infer(&func.return_type);

        self.lower_body(&func.body);
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

    fn lower_closure(&mut self, func: &TypedFunction, func_id: FunctionId, gc_mode: GcMode) {
        let type_params = self.lower_type_params(&func.type_params);

        let env_name = format!("__closure_env_{}", func.name);
        let env_fields: Vec<AirStructField> = func
            .captures
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

        for (cap_name, cap_ty) in &func.captures {
            let local_id =
                self.alloc_named_local(cap_name, self.lower_type_from_infer(cap_ty), false, None);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(local_id),
                    rvalue: Rvalue::FieldAccess {
                        base: Operand::Copy(env_param_id),
                        field: cap_name.clone(),
                    },
                },
                None,
            );
        }

        let user_params = self.lower_params(&func.params);
        let ret_ty = self.lower_type_from_infer(&func.return_type);

        self.lower_body(&func.body);
        self.finalize_function_body();
        self.resolve_block_aliases();

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

    fn lower_params(&mut self, params: &[TypedParam]) -> Vec<AirParam> {
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

    fn func_attribs(&self, func: &TypedFunction) -> FunctionAttribs {
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

    // ========================================================================
    // Top-level statements (global initializers)
    // ========================================================================

    fn lower_toplevel_stmt(&mut self, stmt: &TypedStmt) {
        if let TypedStmtKind::Let {
            name,
            initializer,
            var_type,
            ..
        } = &stmt.kind
        {
            let ty = self.lower_type_from_infer(var_type);
            let init = self.try_const_expr(initializer);
            self.globals.push(AirGlobal {
                name: name.clone(),
                ty,
                init,
                gc_mode: self.file_gc_mode,
                span: Some(self.span(&stmt.span)),
            });
        }
    }

    fn try_const_expr(&self, expr: &TypedExpr) -> Option<AirConst> {
        match &expr.kind {
            TypedExprKind::Int(v) => {
                if expr.ty.is_integer() {
                    Some(AirConst::Int(*v, infer_to_int_size(&expr.ty)))
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
            _ => None,
        }
    }

    // ========================================================================
    // Body lowering
    // ========================================================================

    fn lower_body(&mut self, stmts: &[TypedStmt]) {
        for stmt in stmts {
            self.lower_stmt(stmt);
        }
    }

    fn finalize_function_body(&mut self) {
        if (self.current_stmts.is_empty() && self.current_blocks.is_empty())
            || !self.current_stmts.is_empty()
        {
            self.seal_block(AirTerminator::Return(None));
        }
    }

    fn lower_stmt(&mut self, stmt: &TypedStmt) {
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
                let local = self.alloc_named_local(name, ty, *mutable, sp);
                let operand = self.lower_expr(initializer);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(local),
                        rvalue: Rvalue::Use(operand),
                    },
                    sp,
                );
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
                let operand = val.as_ref().map(|e| self.lower_expr(e));
                self.seal_block(AirTerminator::Return(operand));
            }
            TypedStmtKind::Break => {
                if let Some(loop_ctx) = self.loop_stack.last() {
                    let exit = loop_ctx.exit;
                    self.seal_block(AirTerminator::Goto(exit));
                }
            }
            TypedStmtKind::Continue => {
                if let Some(loop_ctx) = self.loop_stack.last() {
                    let header = loop_ctx.header;
                    self.seal_block(AirTerminator::Goto(header));
                }
            }
            TypedStmtKind::Function(func) => {
                self.lower_function(func);
            }
            TypedStmtKind::Needs(_) | TypedStmtKind::StructDecl { .. } => {}
        }
    }

    // control flow desuccrage
    fn lower_if(
        &mut self,
        condition: &TypedExpr,
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

        self.lower_stmt(then_branch);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(merge_id));
        }
        self.fixup_block_id(then_id);

        if let Some(else_br) = else_branch {
            self.lower_stmt(else_br);
            if !self.last_block_is_terminated() {
                self.seal_block(AirTerminator::Goto(merge_id));
            }
            self.fixup_block_id(else_id);
        }

        self.fixup_block_id_noop(merge_id);
    }

    fn lower_while(&mut self, condition: &TypedExpr, body: &TypedStmt, _sp: Option<Span>) {
        let header_id = self.alloc_block_id();
        let body_id = self.alloc_block_id();
        let exit_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Goto(header_id));

        let cond = self.lower_expr(condition);
        self.seal_block(AirTerminator::Branch {
            cond,
            then_block: body_id,
            else_block: exit_id,
        });
        self.fixup_block_id(header_id);

        self.loop_stack.push(LoopBlocks {
            header: header_id,
            exit: exit_id,
        });
        self.lower_stmt(body);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(header_id));
        }
        self.fixup_block_id(body_id);
        self.loop_stack.pop();

        self.fixup_block_id_noop(exit_id);
    }

    fn lower_for(
        &mut self,
        iterator: &str,
        start: &TypedExpr,
        end: &TypedExpr,
        inclusive: bool,
        step: &Option<TypedExpr>,
        body: &TypedStmt,
    ) {
        let start_span = Some(self.span(&start.span));
        let iter_ty = self.lower_type_from_infer(&start.ty);
        let iter_local = self.alloc_named_local(iterator, iter_ty.clone(), true, start_span);
        let start_op = self.lower_expr(start);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(iter_local),
                rvalue: Rvalue::Use(start_op),
            },
            start_span,
        );

        let end_local = self.alloc_temp(iter_ty.clone());
        let end_op = self.lower_expr(end);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(end_local),
                rvalue: Rvalue::Use(end_op),
            },
            Some(self.span(&end.span)),
        );

        let header_id = self.alloc_block_id();
        let body_id = self.alloc_block_id();
        let incr_id = self.alloc_block_id();
        let exit_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Goto(header_id));

        let cmp_op = if inclusive { BinOp::Le } else { BinOp::Lt };
        let cond_local = self.alloc_temp(AirType::Bool);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(cond_local),
                rvalue: Rvalue::BinaryOp(
                    cmp_op,
                    Operand::Copy(iter_local),
                    Operand::Copy(end_local),
                ),
            },
            None,
        );
        self.seal_block(AirTerminator::Branch {
            cond: Operand::Copy(cond_local),
            then_block: body_id,
            else_block: exit_id,
        });
        self.fixup_block_id(header_id);

        self.loop_stack.push(LoopBlocks {
            header: incr_id,
            exit: exit_id,
        });
        self.lower_stmt(body);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(incr_id));
        }
        self.fixup_block_id(body_id);
        self.loop_stack.pop();

        let step_operand = if let Some(step_expr) = step {
            self.lower_expr(step_expr)
        } else {
            Operand::Const(AirConst::IntLiteral(1))
        };
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(iter_local),
                rvalue: Rvalue::BinaryOp(BinOp::Add, Operand::Copy(iter_local), step_operand),
            },
            None,
        );
        self.seal_block(AirTerminator::Goto(header_id));
        self.fixup_block_id(incr_id);

        self.fixup_block_id_noop(exit_id);
    }

    fn lower_foreach(
        &mut self,
        iterator: &str,
        iterable: &TypedExpr,
        elem_type: &InferType,
        body: &TypedStmt,
        sp: Option<Span>,
    ) {
        let collection = self.lower_expr(iterable);
        let col_ty = self.lower_type_from_infer(&iterable.ty);
        let col_local = self.alloc_temp(col_ty);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(col_local),
                rvalue: Rvalue::Use(collection),
            },
            sp,
        );

        let idx_local = self.alloc_temp(AirType::I64);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(idx_local),
                rvalue: Rvalue::Use(Operand::Const(AirConst::IntLiteral(0))),
            },
            None,
        );

        let len_local = self.alloc_temp(AirType::I64);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(len_local),
                rvalue: Rvalue::Call {
                    func: Callee::Named("__aelys_len".to_string()),
                    args: vec![Operand::Copy(col_local)],
                },
            },
            None,
        );

        let elem_air_ty = self.lower_type_from_infer(elem_type);
        let elem_local = self.alloc_named_local(iterator, elem_air_ty, false, sp);

        let header_id = self.alloc_block_id();
        let body_id = self.alloc_block_id();
        let incr_id = self.alloc_block_id();
        let exit_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Goto(header_id));

        let cond_local = self.alloc_temp(AirType::Bool);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(cond_local),
                rvalue: Rvalue::BinaryOp(
                    BinOp::Lt,
                    Operand::Copy(idx_local),
                    Operand::Copy(len_local),
                ),
            },
            None,
        );
        self.seal_block(AirTerminator::Branch {
            cond: Operand::Copy(cond_local),
            then_block: body_id,
            else_block: exit_id,
        });
        self.fixup_block_id(header_id);

        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(elem_local),
                rvalue: Rvalue::Call {
                    func: Callee::Named("__aelys_index".to_string()),
                    args: vec![Operand::Copy(col_local), Operand::Copy(idx_local)],
                },
            },
            None,
        );

        self.loop_stack.push(LoopBlocks {
            header: incr_id,
            exit: exit_id,
        });
        self.lower_stmt(body);
        if !self.last_block_is_terminated() {
            self.seal_block(AirTerminator::Goto(incr_id));
        }
        self.fixup_block_id(body_id);
        self.loop_stack.pop();

        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(idx_local),
                rvalue: Rvalue::BinaryOp(
                    BinOp::Add,
                    Operand::Copy(idx_local),
                    Operand::Const(AirConst::IntLiteral(1)),
                ),
            },
            None,
        );
        self.seal_block(AirTerminator::Goto(header_id));
        self.fixup_block_id(incr_id);

        self.fixup_block_id_noop(exit_id);
    }

    // ========================================================================
    // Block ID fixup helpers
    //
    // We pre-allocate block IDs, then seal blocks in order.
    // After sealing, the block gets the next sequential ID. We fix it up to
    // the pre-allocated ID so branch targets stay valid.
    // ========================================================================

    fn fixup_block_id(&mut self, target: BlockId) {
        if let Some(block) = self.current_blocks.last_mut() {
            let old_id = block.id;
            block.id = target;
            if old_id != target {
                self.block_aliases.push((old_id.0, target.0));
            }
        }
    }

    fn fixup_block_id_noop(&mut self, target: BlockId) {
        self.pending_block_id = Some(target);
    }

    fn resolve_block_aliases(&mut self) {
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

    fn last_block_is_terminated(&self) -> bool {
        self.current_stmts.is_empty()
            && self.current_blocks.last().is_some_and(|b| {
                !matches!(b.terminator, AirTerminator::Goto(_))
                    || matches!(b.terminator, AirTerminator::Return(_))
                    || matches!(b.terminator, AirTerminator::Unreachable)
            })
    }

    fn lower_expr(&mut self, expr: &TypedExpr) -> Operand {
        let sp = Some(self.span(&expr.span));
        match &expr.kind {
            TypedExprKind::Int(v) => {
                if expr.ty.is_integer() {
                    Operand::Const(AirConst::Int(*v, infer_to_int_size(&expr.ty)))
                } else {
                    Operand::Const(AirConst::IntLiteral(*v))
                }
            }
            TypedExprKind::Float(v) => {
                let size = if matches!(expr.ty, InferType::F32) {
                    AirFloatSize::F32
                } else {
                    AirFloatSize::F64
                };
                Operand::Const(AirConst::Float(*v, size))
            }
            TypedExprKind::Bool(v) => Operand::Const(AirConst::Bool(*v)),
            TypedExprKind::String(v) => Operand::Const(AirConst::Str(v.clone())),
            TypedExprKind::Null => Operand::Const(AirConst::Null),

            TypedExprKind::Identifier(name) => {
                if let Some(id) = self.lookup_local(name) {
                    Operand::Copy(id)
                } else {
                    let tmp = self.alloc_temp(self.lower_type_from_infer(&expr.ty));
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Local(tmp),
                            rvalue: Rvalue::Call {
                                func: Callee::Named(format!("__aelys_global_get_{}", name)),
                                args: Vec::new(),
                            },
                        },
                        sp,
                    );
                    Operand::Copy(tmp)
                }
            }

            TypedExprKind::Binary { left, op, right } => {
                let l = self.lower_expr(left);
                let r = self.lower_expr(right);
                let air_op = lower_binop(op);
                let result_ty = self.lower_type_from_infer(&expr.ty);
                let tmp = self.alloc_temp(result_ty);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::BinaryOp(air_op, l, r),
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }

            TypedExprKind::Unary { op, operand } => {
                let inner = self.lower_expr(operand);
                let air_op = lower_unop(op);
                let result_ty = self.lower_type_from_infer(&expr.ty);
                let tmp = self.alloc_temp(result_ty);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::UnaryOp(air_op, inner),
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }

            TypedExprKind::And { left, right } => self.lower_short_circuit(left, right, true, expr),

            TypedExprKind::Or { left, right } => self.lower_short_circuit(left, right, false, expr),

            TypedExprKind::Call { callee, args } => {
                let lowered_args: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let func = self.lower_callee(callee);
                let result_ty = self.lower_type_from_infer(&expr.ty);
                let tmp = self.alloc_temp(result_ty);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::Call {
                            func,
                            args: lowered_args,
                        },
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }

            TypedExprKind::Assign { name, value } => {
                let val = self.lower_expr(value);
                if let Some(id) = self.lookup_local(name) {
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Local(id),
                            rvalue: Rvalue::Use(val),
                        },
                        sp,
                    );
                    Operand::Copy(id)
                } else {
                    self.emit(
                        AirStmtKind::CallVoid {
                            func: Callee::Named(format!("__aelys_global_set_{}", name)),
                            args: vec![val],
                        },
                        sp,
                    );
                    Operand::Const(AirConst::Null)
                }
            }

            TypedExprKind::Grouping(inner) => self.lower_expr(inner),

            TypedExprKind::If {
                condition,
                then_branch,
                else_branch,
            } => self.lower_if_expr(condition, then_branch, else_branch, expr),

            TypedExprKind::Lambda(inner) => self.lower_expr(inner),

            TypedExprKind::LambdaInner {
                params,
                return_type,
                body,
                captures,
            } => self.lower_lambda(params, return_type, body, captures, expr),

            TypedExprKind::FmtString(parts) => self.lower_fmt_string(parts, sp),

            TypedExprKind::Member { object, member } => {
                let base = self.lower_expr(object);
                let result_ty = self.lower_type_from_infer(&expr.ty);
                let tmp = self.alloc_temp(result_ty);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::FieldAccess {
                            base,
                            field: member.clone(),
                        },
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }

            TypedExprKind::StructLiteral { name, fields } => {
                let lowered_fields: Vec<(String, Operand)> = fields
                    .iter()
                    .map(|(fname, fval)| (fname.clone(), self.lower_expr(fval)))
                    .collect();
                let result_ty = self.lower_type_from_infer(&expr.ty);
                let tmp = self.alloc_temp(result_ty);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::StructInit {
                            name: name.clone(),
                            fields: lowered_fields,
                        },
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }

            TypedExprKind::ArrayLiteral { elements, .. } => {
                let lowered: Vec<Operand> = elements.iter().map(|e| self.lower_expr(e)).collect();
                let tmp = self.alloc_temp(self.lower_type_from_infer(&expr.ty));
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::Call {
                            func: Callee::Named("__aelys_array_new".to_string()),
                            args: lowered,
                        },
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }

            TypedExprKind::ArraySized { size, .. } => {
                let sz = self.lower_expr(size);
                let tmp = self.alloc_temp(self.lower_type_from_infer(&expr.ty));
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::Call {
                            func: Callee::Named("__aelys_array_sized".to_string()),
                            args: vec![sz],
                        },
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }

            TypedExprKind::VecLiteral { elements, .. } => {
                let lowered: Vec<Operand> = elements.iter().map(|e| self.lower_expr(e)).collect();
                let tmp = self.alloc_temp(self.lower_type_from_infer(&expr.ty));
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::Call {
                            func: Callee::Named("__aelys_vec_new".to_string()),
                            args: lowered,
                        },
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }

            TypedExprKind::Index { object, index } => {
                let obj = self.lower_expr(object);
                let idx = self.lower_expr(index);
                let result_ty = self.lower_type_from_infer(&expr.ty);
                let tmp = self.alloc_temp(result_ty);
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::Call {
                            func: Callee::Named("__aelys_index".to_string()),
                            args: vec![obj, idx],
                        },
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }

            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
                let obj = self.lower_expr(object);
                let idx = self.lower_expr(index);
                let val = self.lower_expr(value);
                self.emit(
                    AirStmtKind::CallVoid {
                        func: Callee::Named("__aelys_index_set".to_string()),
                        args: vec![obj, idx, val],
                    },
                    sp,
                );
                Operand::Const(AirConst::Null)
            }

            TypedExprKind::Range { start, end, .. } => {
                let mut args = Vec::new();
                if let Some(s) = start {
                    args.push(self.lower_expr(s));
                }
                if let Some(e) = end {
                    args.push(self.lower_expr(e));
                }
                let tmp = self.alloc_temp(self.lower_type_from_infer(&expr.ty));
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::Call {
                            func: Callee::Named("__aelys_range".to_string()),
                            args,
                        },
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }

            TypedExprKind::Slice { object, range } => {
                let obj = self.lower_expr(object);
                let rng = self.lower_expr(range);
                let tmp = self.alloc_temp(self.lower_type_from_infer(&expr.ty));
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::Call {
                            func: Callee::Named("__aelys_slice".to_string()),
                            args: vec![obj, rng],
                        },
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }

            TypedExprKind::Cast {
                expr: inner,
                target,
            } => {
                let operand = self.lower_expr(inner);
                let from = self.lower_type_from_infer(&inner.ty);
                let to = self.lower_type_from_infer(target);
                let tmp = self.alloc_temp(to.clone());
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::Cast { operand, from, to },
                    },
                    sp,
                );
                Operand::Copy(tmp)
            }
        }
    }

    fn lower_expr_discard(&mut self, expr: &TypedExpr) {
        let sp = Some(self.span(&expr.span));
        match &expr.kind {
            TypedExprKind::Call { callee, args } => {
                let lowered_args: Vec<Operand> = args.iter().map(|a| self.lower_expr(a)).collect();
                let func = self.lower_callee(callee);
                if matches!(expr.ty, InferType::Null) {
                    self.emit(
                        AirStmtKind::CallVoid {
                            func,
                            args: lowered_args,
                        },
                        sp,
                    );
                } else {
                    let tmp = self.alloc_temp(self.lower_type_from_infer(&expr.ty));
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Local(tmp),
                            rvalue: Rvalue::Call {
                                func,
                                args: lowered_args,
                            },
                        },
                        sp,
                    );
                }
            }
            TypedExprKind::Assign { name, value } => {
                let val = self.lower_expr(value);
                if let Some(id) = self.lookup_local(name) {
                    self.emit(
                        AirStmtKind::Assign {
                            place: Place::Local(id),
                            rvalue: Rvalue::Use(val),
                        },
                        sp,
                    );
                } else {
                    self.emit(
                        AirStmtKind::CallVoid {
                            func: Callee::Named(format!("__aelys_global_set_{}", name)),
                            args: vec![val],
                        },
                        sp,
                    );
                }
            }
            TypedExprKind::IndexAssign {
                object,
                index,
                value,
            } => {
                let obj = self.lower_expr(object);
                let idx = self.lower_expr(index);
                let val = self.lower_expr(value);
                self.emit(
                    AirStmtKind::CallVoid {
                        func: Callee::Named("__aelys_index_set".to_string()),
                        args: vec![obj, idx, val],
                    },
                    sp,
                );
            }
            _ => {
                self.lower_expr(expr);
            }
        }
    }

    fn lower_callee(&mut self, callee: &TypedExpr) -> Callee {
        match &callee.kind {
            TypedExprKind::Identifier(name) => Callee::Named(name.clone()),
            TypedExprKind::Member { object, member } => {
                if let TypedExprKind::Identifier(mod_name) = &object.kind {
                    Callee::Named(format!("{}.{}", mod_name, member))
                } else {
                    let op = self.lower_expr(callee);
                    let tmp = match op {
                        Operand::Copy(id) | Operand::Move(id) => id,
                        Operand::Const(_) => {
                            let t = self.alloc_temp(self.lower_type_from_infer(&callee.ty));
                            self.emit(
                                AirStmtKind::Assign {
                                    place: Place::Local(t),
                                    rvalue: Rvalue::Use(op),
                                },
                                None,
                            );
                            t
                        }
                    };
                    Callee::FnPtr(tmp)
                }
            }
            _ => {
                let op = self.lower_expr(callee);
                let tmp = match op {
                    Operand::Copy(id) | Operand::Move(id) => id,
                    Operand::Const(_) => {
                        let t = self.alloc_temp(self.lower_type_from_infer(&callee.ty));
                        self.emit(
                            AirStmtKind::Assign {
                                place: Place::Local(t),
                                rvalue: Rvalue::Use(op),
                            },
                            None,
                        );
                        t
                    }
                };
                Callee::FnPtr(tmp)
            }
        }
    }

    // short-circuit lowering (and/or)
    fn lower_short_circuit(
        &mut self,
        left: &TypedExpr,
        right: &TypedExpr,
        is_and: bool,
        _parent: &TypedExpr,
    ) -> Operand {
        let result = self.alloc_temp(AirType::Bool);
        let lhs = self.lower_expr(left);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(result),
                rvalue: Rvalue::Use(lhs),
            },
            None,
        );

        let eval_right_id = self.alloc_block_id();
        let merge_id = self.alloc_block_id();

        if is_and {
            self.seal_block(AirTerminator::Branch {
                cond: Operand::Copy(result),
                then_block: eval_right_id,
                else_block: merge_id,
            });
        } else {
            self.seal_block(AirTerminator::Branch {
                cond: Operand::Copy(result),
                then_block: merge_id,
                else_block: eval_right_id,
            });
        }

        let rhs = self.lower_expr(right);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(result),
                rvalue: Rvalue::Use(rhs),
            },
            None,
        );
        self.seal_block(AirTerminator::Goto(merge_id));
        self.fixup_block_id(eval_right_id);

        self.fixup_block_id_noop(merge_id);
        Operand::Copy(result)
    }

    // ========================================================================
    // If-expression lowering
    // ========================================================================

    fn lower_if_expr(
        &mut self,
        condition: &TypedExpr,
        then_branch: &TypedExpr,
        else_branch: &TypedExpr,
        parent: &TypedExpr,
    ) -> Operand {
        let result_ty = self.lower_type_from_infer(&parent.ty);
        let result = self.alloc_temp(result_ty);

        let cond = self.lower_expr(condition);
        let then_id = self.alloc_block_id();
        let else_id = self.alloc_block_id();
        let merge_id = self.alloc_block_id();

        self.seal_block(AirTerminator::Branch {
            cond,
            then_block: then_id,
            else_block: else_id,
        });

        let then_val = self.lower_expr(then_branch);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(result),
                rvalue: Rvalue::Use(then_val),
            },
            None,
        );
        self.seal_block(AirTerminator::Goto(merge_id));
        self.fixup_block_id(then_id);

        let else_val = self.lower_expr(else_branch);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(result),
                rvalue: Rvalue::Use(else_val),
            },
            None,
        );
        self.seal_block(AirTerminator::Goto(merge_id));
        self.fixup_block_id(else_id);

        self.fixup_block_id_noop(merge_id);
        Operand::Copy(result)
    }

    // lamba lowering (desugarded to closure env struct + function)
    fn lower_lambda(
        &mut self,
        params: &[TypedParam],
        return_type: &InferType,
        body: &[TypedStmt],
        captures: &[(String, InferType)],
        parent: &TypedExpr,
    ) -> Operand {
        let lambda_name = format!("__lambda_{}", self.next_function_id);
        let fake_func = TypedFunction {
            name: lambda_name.clone(),
            type_params: Vec::new(),
            params: params.to_vec(),
            return_type: return_type.clone(),
            body: body.to_vec(),
            decorators: Vec::new(),
            is_pub: false,
            span: parent.span,
            captures: captures.to_vec(),
        };
        self.lower_function(&fake_func);

        let result_ty = self.lower_type_from_infer(&parent.ty);
        let tmp = self.alloc_temp(result_ty);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(tmp),
                rvalue: Rvalue::Use(Operand::Const(AirConst::Null)),
            },
            Some(self.span(&parent.span)),
        );
        Operand::Copy(tmp)
    }

    // format string → __aelys_str_concat / __aelys_to_string
    fn lower_fmt_string(&mut self, parts: &[TypedFmtStringPart], sp: Option<Span>) -> Operand {
        let mut operands: Vec<Operand> = Vec::new();

        for part in parts {
            match part {
                TypedFmtStringPart::Literal(s) => {
                    operands.push(Operand::Const(AirConst::Str(s.clone())));
                }
                TypedFmtStringPart::Expr(expr) => {
                    let val = self.lower_expr(expr);
                    if matches!(expr.ty, InferType::String) {
                        operands.push(val);
                    } else {
                        let str_tmp = self.alloc_temp(AirType::Str);
                        self.emit(
                            AirStmtKind::Assign {
                                place: Place::Local(str_tmp),
                                rvalue: Rvalue::Call {
                                    func: Callee::Named("__aelys_to_string".to_string()),
                                    args: vec![val],
                                },
                            },
                            None,
                        );
                        operands.push(Operand::Copy(str_tmp));
                    }
                }
                TypedFmtStringPart::Placeholder => {
                    operands.push(Operand::Const(AirConst::Str(String::new())));
                }
            }
        }

        if operands.is_empty() {
            return Operand::Const(AirConst::Str(String::new()));
        }
        if operands.len() == 1 {
            return operands.into_iter().next().unwrap();
        }

        let mut acc = operands.remove(0);
        for part in operands {
            let tmp = self.alloc_temp(AirType::Str);
            self.emit(
                AirStmtKind::Assign {
                    place: Place::Local(tmp),
                    rvalue: Rvalue::Call {
                        func: Callee::Named("__aelys_str_concat".to_string()),
                        args: vec![acc, part],
                    },
                },
                sp,
            );
            acc = Operand::Copy(tmp);
        }
        acc
    }
}

// helpers
fn infer_to_int_size(ty: &InferType) -> AirIntSize {
    match ty {
        InferType::I8 => AirIntSize::I8,
        InferType::I16 => AirIntSize::I16,
        InferType::I32 => AirIntSize::I32,
        InferType::I64 => AirIntSize::I64,
        InferType::U8 => AirIntSize::U8,
        InferType::U16 => AirIntSize::U16,
        InferType::U32 => AirIntSize::U32,
        InferType::U64 => AirIntSize::U64,
        _ => AirIntSize::I64,
    }
}

fn lower_binop(op: &BinaryOp) -> BinOp {
    match op {
        BinaryOp::Add => BinOp::Add,
        BinaryOp::Sub => BinOp::Sub,
        BinaryOp::Mul => BinOp::Mul,
        BinaryOp::Div => BinOp::Div,
        BinaryOp::Mod => BinOp::Rem,
        BinaryOp::Eq => BinOp::Eq,
        BinaryOp::Ne => BinOp::Ne,
        BinaryOp::Lt => BinOp::Lt,
        BinaryOp::Le => BinOp::Le,
        BinaryOp::Gt => BinOp::Gt,
        BinaryOp::Ge => BinOp::Ge,
        BinaryOp::Shl => BinOp::Shl,
        BinaryOp::Shr => BinOp::Shr,
        BinaryOp::BitAnd => BinOp::BitAnd,
        BinaryOp::BitOr => BinOp::BitOr,
        BinaryOp::BitXor => BinOp::BitXor,
    }
}

fn lower_unop(op: &aelys_syntax::UnaryOp) -> UnOp {
    match op {
        aelys_syntax::UnaryOp::Neg => UnOp::Neg,
        aelys_syntax::UnaryOp::Not => UnOp::Not,
        aelys_syntax::UnaryOp::BitNot => UnOp::BitNot,
    }
}

mod expr;
mod loops;
mod place;
mod program;
mod stmts;

use crate::*;
use aelys_sema::{InferType, TypedProgram};
use aelys_syntax::BinaryOp;

pub fn lower(program: &TypedProgram) -> AirProgram {
    try_lower(program).unwrap_or_else(|failure| panic!("{}", format_lowering_errors(&failure)))
}

// a borrow rejection renders richly (spans/carets/codes); an air-lowering failure keeps its flat
pub enum LowerFailure {
    Borrow(Vec<crate::bir::BirDiagnostic>),
    Lowering(Vec<String>),
}

fn build_and_check_bir(
    program: &TypedProgram,
) -> Result<
    std::collections::HashMap<crate::bir::DropKey, Vec<crate::bir::DropKey>>,
    Vec<crate::bir::BirDiagnostic>,
> {
    let bir = crate::bir::build::build_program(program);
    let check = crate::bir::check_program(&bir);
    if check.errors.is_empty() {
        Ok(check.drops)
    } else {
        Err(check.errors)
    }
}

pub fn try_lower(program: &TypedProgram) -> Result<AirProgram, LowerFailure> {
    let drops = build_and_check_bir(program).map_err(LowerFailure::Borrow)?;
    let mut cx = LoweringContext::new(program);
    cx.affine_drops = drops;
    cx.lower_program();
    cx.finish().map_err(LowerFailure::Lowering)
}

pub fn lower_with_gc_mode(program: &TypedProgram, file_gc_mode: GcMode) -> AirProgram {
    try_lower_with_gc_mode(program, file_gc_mode)
        .unwrap_or_else(|failure| panic!("{}", format_lowering_errors(&failure)))
}

pub fn try_lower_with_gc_mode(
    program: &TypedProgram,
    file_gc_mode: GcMode,
) -> Result<AirProgram, LowerFailure> {
    let drops = build_and_check_bir(program).map_err(LowerFailure::Borrow)?;
    let mut cx = LoweringContext::new(program);
    cx.file_gc_mode = file_gc_mode;
    cx.affine_drops = drops;
    cx.lower_program();
    cx.finish().map_err(LowerFailure::Lowering)
}

pub(crate) struct LoweringContext<'a> {
    pub(super) program: &'a TypedProgram,
    pub(super) functions: Vec<AirFunction>,
    pub(super) structs: Vec<AirStructDef>,
    pub(super) enums: Vec<AirEnumDef>,
    pub(super) globals: Vec<AirGlobal>,
    pub(super) source_files: Vec<String>,
    pub(super) next_function_id: u32,
    pub(super) next_local_id: u32,
    pub(super) next_block_id: u32,
    pub(super) file_gc_mode: GcMode,
    pub(super) current_blocks: Vec<AirBlock>,
    pub(super) current_locals: Vec<AirLocal>,
    pub(super) current_params: Vec<AirParam>,
    pub(super) current_stmts: Vec<AirStmt>,
    pub(super) locals_by_name: Vec<(String, LocalId)>,
    // these three registries are all fed from the sema type, never the AIR type, which
    // erases an Rc into a plain Ptr indistinguishable from a closure env or a null
    pub(super) rc_locals: Vec<(LocalId, usize)>,
    // never fed from lower_params: releasing a borrowed carrier param callee-side would
    // hand the caller a use-after-free
    pub(super) carrier_locals: Vec<CarrierLocal>,
    pub(super) cow_locals: Vec<(LocalId, usize)>,
    // no drop bool: the point-keyed plan below decides every drop, read per program point.
    pub(super) affine_locals: Vec<AffineLocal>,
    pub(super) affine_drops:
        std::collections::HashMap<crate::bir::DropKey, Vec<crate::bir::DropKey>>,
    pub(super) loop_stack: Vec<LoopBlocks>,
    pub(super) type_params_map: Vec<(String, TypeParamId)>,
    pub(super) pending_block_id: Option<BlockId>,
    pub(super) block_aliases: Vec<(u32, u32)>,
    pub(super) closure_env_param: Option<LocalId>,
    pub(super) capture_slots: std::collections::HashMap<LocalId, String>,
    pub(super) lowering_errors: Vec<String>,
}

pub(super) struct CarrierLocal {
    pub(super) local: LocalId,
    pub(super) ty: AirType,
    pub(super) paths: Vec<crate::rc_paths::RcLeafPath>,
    pub(super) depth: usize,
}

// a live affine local, dropped point-sensitively per the plan; decl_key is the cross-ir key
pub(super) struct AffineLocal {
    pub(super) local: LocalId,
    pub(super) depth: usize,
    pub(super) id_field: String,
    pub(super) decl_key: crate::bir::DropKey,
}

pub(super) struct LoopBlocks {
    pub(super) header: BlockId,
    pub(super) exit: BlockId,
    // captured at loop entry, to catch an Rc born in the body that a break would abandon
    pub(super) body_scope_depth: usize,
}

impl<'a> LoweringContext<'a> {
    fn new(program: &'a TypedProgram) -> Self {
        Self {
            program,
            functions: Vec::new(),
            structs: Vec::new(),
            enums: Vec::new(),
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
            rc_locals: Vec::new(),
            carrier_locals: Vec::new(),
            cow_locals: Vec::new(),
            affine_locals: Vec::new(),
            affine_drops: std::collections::HashMap::new(),
            loop_stack: Vec::new(),
            type_params_map: Vec::new(),
            pending_block_id: None,
            block_aliases: Vec::new(),
            closure_env_param: None,
            capture_slots: std::collections::HashMap::new(),
            lowering_errors: Vec::new(),
        }
    }

    fn finish(self) -> Result<AirProgram, Vec<String>> {
        if !self.lowering_errors.is_empty() {
            return Err(self.lowering_errors);
        }
        Ok(AirProgram {
            functions: self.functions,
            structs: self.structs,
            enums: self.enums,
            globals: self.globals,
            source_files: self.source_files,
            mono_instances: Vec::new(),
            struct_sizes: std::collections::HashMap::new(),
            rc_type_table: crate::rc_types::RcTypeTable::default(),
        })
    }

    pub(super) fn alloc_function_id(&mut self) -> FunctionId {
        let id = FunctionId(self.next_function_id);
        self.next_function_id += 1;
        id
    }

    pub(super) fn alloc_local_id(&mut self) -> LocalId {
        let id = LocalId(self.next_local_id);
        self.next_local_id += 1;
        id
    }

    pub(super) fn alloc_block_id(&mut self) -> BlockId {
        let id = BlockId(self.next_block_id);
        self.next_block_id += 1;
        id
    }

    pub(super) fn alloc_temp(&mut self, ty: AirType) -> LocalId {
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

    // codegen keeps a flat value map that ignores SSA dominance, so anything written from
    // two or more blocks needs a real alloca, which is what mut gives it
    pub(super) fn alloc_temp_mut(&mut self, ty: AirType) -> LocalId {
        let id = self.alloc_local_id();
        self.current_locals.push(AirLocal {
            id,
            ty,
            name: None,
            is_mut: true,
            span: None,
        });
        id
    }

    pub(super) fn alloc_named_local(
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

    // the capture prologue allocates its pointer through place.rs, which cannot name it, so
    pub(super) fn rename_local(&mut self, id: LocalId, name: &str) {
        if let Some(l) = self.current_locals.iter_mut().find(|l| l.id == id) {
            l.name = Some(name.to_string());
        }
        self.locals_by_name.push((name.to_string(), id));
    }

    pub(super) fn lookup_local(&self, name: &str) -> Option<LocalId> {
        self.locals_by_name
            .iter()
            .rev()
            .find(|(n, _)| n == name)
            .map(|(_, id)| *id)
    }

    pub(super) fn emit(&mut self, kind: AirStmtKind, span: Option<Span>) {
        self.current_stmts.push(AirStmt { kind, span });
    }

    pub(super) fn seal_block(&mut self, terminator: AirTerminator) -> BlockId {
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

    pub(super) fn span(&self, s: &aelys_syntax::Span) -> Span {
        Span {
            file: 0,
            lo: s.start as u32,
            hi: s.end as u32,
        }
    }

    pub(super) fn emit_rvalue_to_temp(
        &mut self,
        ty: AirType,
        rvalue: Rvalue,
        sp: Option<Span>,
    ) -> Operand {
        let tmp = self.alloc_temp(ty);
        self.emit(
            AirStmtKind::Assign {
                place: Place::Local(tmp),
                rvalue,
            },
            sp,
        );
        Operand::Copy(tmp)
    }

    pub(super) fn operand_to_local(&mut self, op: Operand, ty: &AirType) -> LocalId {
        match op {
            Operand::Copy(id) | Operand::Move(id) => id,
            Operand::Const(_) => {
                let tmp = self.alloc_temp(ty.clone());
                self.emit(
                    AirStmtKind::Assign {
                        place: Place::Local(tmp),
                        rvalue: Rvalue::Use(op),
                    },
                    None,
                );
                tmp
            }
        }
    }

    pub(super) fn local_air_type(&self, id: LocalId) -> Option<AirType> {
        self.current_locals
            .iter()
            .find(|l| l.id == id)
            .map(|l| l.ty.clone())
    }

    pub(super) fn report_error(&mut self, message: String) {
        self.lowering_errors.push(message);
    }

    pub(super) fn lower_type_params(&mut self, type_params: &[String]) -> Vec<TypeParamId> {
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

    pub(super) fn lower_type_from_infer(&self, ty: &InferType) -> AirType {
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
            InferType::Null => AirType::Ptr(Box::new(AirType::Void)),
            InferType::Function { params, ret, .. } => AirType::FnPtr {
                params: params
                    .iter()
                    .map(|p| self.lower_type_from_infer(p))
                    .collect(),
                ret: Box::new(self.lower_type_from_infer(ret)),
                conv: CallingConv::Aelys,
            },
            InferType::Array(inner, Some(n)) => {
                AirType::Array(Box::new(self.lower_type_from_infer(inner)), *n)
            }
            InferType::Array(inner, None) => {
                AirType::Slice(Box::new(self.lower_type_from_infer(inner)))
            }
            // a Vec is its own 24-byte type, not the 16-byte array-view Slice
            InferType::Vec(inner) => AirType::Vec(Box::new(self.lower_type_from_infer(inner))),
            // an Rc erases to a plain data pointer, the refcount machinery lives in the
            // lowering and the runtime, never in the type
            InferType::Rc(inner) => AirType::Ptr(Box::new(self.lower_type_from_infer(inner))),
            // references erase to raw ptr / fat {ptr,len}, mutability is dropped
            InferType::Ref { referent, .. } => {
                AirType::Ptr(Box::new(self.lower_type_from_infer(referent)))
            }
            InferType::Slice { elem, .. } => {
                AirType::Slice(Box::new(self.lower_type_from_infer(elem)))
            }
            // TODO: add support for InferType::Tuple in the backend
            InferType::Tuple(_) => {
                #[cfg(debug_assertions)]
                eprintln!(
                    "[AIR] warning: InferType::Tuple reached lower_type_from_infer, \
                     tuples are not yet supported in the LLVM backend"
                );
                AirType::Opaque
            }
            // TODO: add support for InferType::Range in the backend
            InferType::Range => {
                #[cfg(debug_assertions)]
                eprintln!(
                    "[AIR] warning: InferType::Range reached lower_type_from_infer, \
                     ranges are not yet supported in the LLVM backend"
                );
                AirType::Opaque
            }
            InferType::Struct(name) => {
                if let Some((_, id)) = self.type_params_map.iter().find(|(n, _)| n == name) {
                    AirType::Param(*id)
                } else {
                    AirType::Struct(name.clone())
                }
            }
            InferType::Enum(name, type_args) => {
                if type_args.is_empty() {
                    AirType::Enum(name.clone())
                } else {
                    // pre-mangle so the mono pass can disambiguate unit variant assignments
                    let lowered_args: Vec<AirType> = type_args
                        .iter()
                        .map(|a| self.lower_type_from_infer(a))
                        .collect();
                    // only when every arg is concrete: an unresolved or generic arg would
                    // mangle to a name monomorphization cannot rewrite
                    let all_concrete = lowered_args
                        .iter()
                        .all(|t| !matches!(t, AirType::Opaque | AirType::Param(_)));
                    if all_concrete {
                        let suffix = lowered_args
                            .iter()
                            .map(|t| crate::mono::substitute::type_to_string(t))
                            .collect::<Vec<_>>()
                            .join("$");
                        AirType::Enum(format!("__mono_{}_{}", name, suffix))
                    } else {
                        AirType::Enum(name.clone())
                    }
                }
            }
            // a Var reaching lowering is a compiler bug, finalize should have widened it;
            // map it to Opaque so validation rejects it with a clear message
            InferType::Var(_id) => {
                #[cfg(debug_assertions)]
                eprintln!(
                    "[AIR] ICE: InferType::Var({}) leaked past finalization into lower_type_from_infer",
                    _id.0
                );
                AirType::Opaque
            }
            InferType::Never => AirType::Void,
            // mono patches this for generic call results; anything else stays Opaque and is
            // rejected by validation
            InferType::Dynamic => AirType::Opaque,
        }
    }

    pub(super) fn check_stack_array_size(&mut self, elem_ty: &AirType, n: u64) {
        const MAX_STACK_BYTES: u64 = 1024 * 1024; // 1 MB
        let elem_size = self.stack_array_elem_size(elem_ty) as u64;
        let total = n.saturating_mul(elem_size);
        if total > MAX_STACK_BYTES {
            self.report_error(format!(
                "stack array too large: [{}; {}] = {} bytes (max {} bytes). \
                 Consider using a smaller size or a heap-allocated collection.",
                crate::print::fmt_type(elem_ty),
                n,
                total,
                MAX_STACK_BYTES,
            ));
        }
    }

    fn stack_array_elem_size(&self, elem_ty: &AirType) -> u32 {
        let mut probe = AirProgram {
            functions: vec![AirFunction {
                id: FunctionId(0),
                name: "__stack_size_probe".to_string(),
                gc_mode: GcMode::Managed,
                type_params: vec![],
                params: vec![],
                ret_ty: AirType::Void,
                locals: vec![AirLocal {
                    id: LocalId(0),
                    ty: elem_ty.clone(),
                    name: Some("__probe".to_string()),
                    is_mut: false,
                    span: None,
                }],
                blocks: vec![],
                is_extern: true,
                calling_conv: CallingConv::Aelys,
                attributes: FunctionAttribs {
                    inline: InlineHint::Default,
                    no_gc: false,
                    no_unwind: false,
                    cold: false,
                },
                span: None,
            }],
            structs: self.structs.clone(),
            enums: self.enums.clone(),
            globals: vec![],
            source_files: vec![],
            mono_instances: vec![],
            struct_sizes: std::collections::HashMap::new(),
            rc_type_table: crate::rc_types::RcTypeTable::default(),
        };

        // layout_of is context-free and undersizes data enums to 4 bytes, so probe instead
        probe = crate::mono::monomorphize(probe)
            .expect("invariant: the compiler-built layout probe always monomorphizes");
        let _ = crate::layout::compute_layouts(&mut probe);
        crate::layout::resolved_layout(elem_ty, &probe.struct_sizes).size
    }

    pub(super) fn gc_mode_for_function(&self, func: &aelys_sema::TypedFunction) -> GcMode {
        if func.decorators.iter().any(|d| d.name == "no_gc") {
            GcMode::Manual
        } else {
            self.file_gc_mode
        }
    }

    pub(super) fn affine_category(&self, ty: &InferType) -> crate::bir::Category {
        crate::bir::category(ty, &self.program.type_table)
    }

    pub(super) fn air_drop_key(sp: Option<Span>) -> Option<crate::bir::DropKey> {
        sp.map(|s| (s.lo as usize, s.hi as usize))
    }

    pub(super) fn emit_affine_drop(&mut self, local: LocalId, id_field: &str, sp: Option<Span>) {
        let base_ty = self.local_air_type(local).unwrap_or(AirType::Struct(
            crate::bir::category::AFFINE_TEST_TYPE.to_string(),
        ));
        let field_ty = self.air_struct_field_type(&base_ty, id_field);
        let id = self.emit_rvalue_to_temp(
            field_ty,
            Rvalue::FieldAccess {
                base: Operand::Copy(local),
                field: id_field.to_string(),
            },
            sp,
        );
        self.emit(
            AirStmtKind::CallVoid {
                func: Callee::Named("println".to_string()),
                args: vec![id],
            },
            sp,
        );
    }

    // collect (local, id_field) to drop at a point: registered affines whose decl is listed by
    pub(super) fn collect_affine_drops(
        &self,
        point_key: crate::bir::DropKey,
        depth_ok: impl Fn(usize) -> bool,
    ) -> Vec<(LocalId, String)> {
        let Some(decls) = self.affine_drops.get(&point_key) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        for decl in decls {
            if let Some(al) = self
                .affine_locals
                .iter()
                .find(|a| a.decl_key == *decl && depth_ok(a.depth))
            {
                out.push((al.local, al.id_field.clone()));
            }
        }
        out
    }
}

fn format_lowering_errors(failure: &LowerFailure) -> String {
    // borrow rejections join their primary phrases (markers preserved) into the same shell as the
    let errors: Vec<String> = match failure {
        LowerFailure::Lowering(errors) => errors.clone(),
        LowerFailure::Borrow(diags) => diags.iter().map(|d| d.primary.1.clone()).collect(),
    };
    let joined = errors
        .iter()
        .enumerate()
        .map(|(i, e)| format!("  {}. {}", i + 1, e))
        .collect::<Vec<_>>()
        .join("\n");
    format!(
        "AIR lowering failed with {} error(s):\n{}",
        errors.len(),
        joined
    )
}

pub(crate) fn infer_to_int_size(ty: &InferType) -> AirIntSize {
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

mod expr;
mod loops;
mod program;
mod stmts;

use crate::*;
use aelys_sema::{InferType, TypedProgram};
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

pub(crate) struct LoweringContext<'a> {
    pub(super) program: &'a TypedProgram,
    pub(super) functions: Vec<AirFunction>,
    pub(super) structs: Vec<AirStructDef>,
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
    pub(super) loop_stack: Vec<LoopBlocks>,
    pub(super) type_params_map: Vec<(String, TypeParamId)>,
    pub(super) pending_block_id: Option<BlockId>,
    pub(super) block_aliases: Vec<(u32, u32)>,
}

pub(super) struct LoopBlocks {
    pub(super) header: BlockId,
    pub(super) exit: BlockId,
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

    // alloc_temp creates immutable locals ->> codegen uses a flat value_map that doesn't respect SSA dominance
    // anything written from 2+ blocks needs an alloca.
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

    /// Alloc a temp, emit an Assign of rvalue into it, return Copy(tmp).
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

    /// Extract a LocalId from an Operand, materializing a const to a temp if needed.
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
            // unresolved type vars reaching lowering = void (no explicit return annotation)
            InferType::Var(_) => AirType::Void,
            // TODO: Terrible, terrible, terrible, did I forgot to say terrible ?
            // Dynamic = sema's "gradual typing" fallback, which we use for generic call results
            // that monomorphization will fix later. i64 placeholder works because
            // monomorphization replaces the type before codegen sees it.
            InferType::Dynamic => AirType::I64,
        }
    }

    pub(super) fn gc_mode_for_function(&self, func: &aelys_sema::TypedFunction) -> GcMode {
        if func.decorators.iter().any(|d| d.name == "no_gc") {
            GcMode::Manual
        } else {
            self.file_gc_mode
        }
    }
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

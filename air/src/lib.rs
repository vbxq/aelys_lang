
pub mod analysis;
pub mod bir;
pub mod layout;
pub mod lower;
pub mod modules;
pub mod mono;
pub mod passes;
pub mod print;
pub mod rc_paths;
pub mod rc_types;
pub mod symbols;

pub use bir::{Checked, check};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct LocalId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct BlockId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FunctionId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ArenaId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeParamId(pub u32);

#[derive(Debug, Clone, Copy)]
pub struct Span {
    pub file: u32,
    pub lo: u32,
    pub hi: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EnumRef {
    pub name: String,
    pub args: Vec<AirType>,
}

impl EnumRef {
    pub fn plain(name: impl Into<String>) -> Self {
        EnumRef {
            name: name.into(),
            args: Vec::new(),
        }
    }

    pub fn new(name: impl Into<String>, args: Vec<AirType>) -> Self {
        EnumRef {
            name: name.into(),
            args,
        }
    }

    // the symbol is produced for codegen and display and is never parsed back for meaning
    pub fn symbol(&self) -> String {
        derive_enum_symbol(&self.name, &self.args)
    }
}

// three pinned strings and the byte golden read a plain enum under its bare name, so arity 0 must not decorate
pub fn derive_enum_symbol(name: &str, args: &[AirType]) -> String {
    if args.is_empty() {
        return name.to_string();
    }
    let rendered = args
        .iter()
        .map(crate::mono::substitute::type_to_string)
        .collect::<Vec<_>>()
        .join("$");
    format!("__mono_{}${}${}", name, args.len(), rendered)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AirType {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    Bool,
    /// byte string slice abi: (ptr, len), never nul-terminated.
    Str,
    Ptr(Box<AirType>),
    Struct(String),
    Enum(EnumRef),
    Array(Box<AirType>, u64),
    Slice(Box<AirType>),
    // extra cap field never perturbs immutable array views
    Vec(Box<AirType>),
    FnPtr {
        params: Vec<AirType>,
        ret: Box<AirType>,
        conv: CallingConv,
    },
    Param(TypeParamId),
    // mono must eliminate this, validation rejects any opaque that survives
    Opaque,
    Void,
}

impl AirType {
    pub fn int_size(&self) -> Option<AirIntSize> {
        match self {
            AirType::I8 => Some(AirIntSize::I8),
            AirType::I16 => Some(AirIntSize::I16),
            AirType::I32 => Some(AirIntSize::I32),
            AirType::I64 => Some(AirIntSize::I64),
            AirType::U8 => Some(AirIntSize::U8),
            AirType::U16 => Some(AirIntSize::U16),
            AirType::U32 => Some(AirIntSize::U32),
            AirType::U64 => Some(AirIntSize::U64),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AirIntSize {
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AirFloatSize {
    F32,
    F64,
}

#[derive(Clone)]
pub struct AirStructDef {
    pub name: String,
    pub type_params: Vec<TypeParamId>,
    pub fields: Vec<AirStructField>,
    pub is_closure_env: bool,
    pub span: Option<Span>,
}

#[derive(Clone)]
pub struct AirStructField {
    pub name: String,
    pub ty: AirType,
    pub offset: Option<u32>,
}

#[derive(Clone)]
pub struct AirEnumVariant {
    pub name: String,
    pub tag: u32,
    pub payload: Vec<AirType>, // empty = unit variant, non-empty = data variant
}

#[derive(Clone)]
pub struct AirEnumDef {
    pub name: String,
    pub type_params: Vec<TypeParamId>,
    pub variants: Vec<AirEnumVariant>,
    pub span: Option<Span>,
}

#[derive(Clone)]
pub struct AirProgram {
    pub functions: Vec<AirFunction>,
    pub structs: Vec<AirStructDef>,
    pub enums: Vec<AirEnumDef>,
    pub globals: Vec<AirGlobal>,
    pub source_files: Vec<String>,
    pub mono_instances: Vec<MonoInstance>,
    pub struct_sizes: std::collections::HashMap<String, layout::TypeLayout>,
    pub rc_type_table: rc_types::RcTypeTable,
}

#[derive(Clone)]
pub struct AirGlobal {
    pub name: String,
    pub ty: AirType,
    pub init: Option<AirConst>,
    pub gc_mode: GcMode,
    pub span: Option<Span>,
}

#[derive(Clone)]
pub struct MonoInstance {
    pub original: FunctionId,
    pub type_args: Vec<AirType>,
    pub result: FunctionId,
}

#[derive(Clone)]
pub struct AirFunction {
    pub id: FunctionId,
    pub name: String,
    pub gc_mode: GcMode,
    pub type_params: Vec<TypeParamId>,
    pub params: Vec<AirParam>,
    pub ret_ty: AirType,
    pub locals: Vec<AirLocal>,
    pub blocks: Vec<AirBlock>,
    pub is_extern: bool,
    pub calling_conv: CallingConv,
    pub attributes: FunctionAttribs,
    pub span: Option<Span>,
}

#[derive(Clone)]
pub struct FunctionAttribs {
    pub inline: InlineHint,
    pub no_gc: bool,
    pub no_unwind: bool,
    pub cold: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InlineHint {
    Default,
    Always,
    Never,
}

#[derive(Clone)]
pub struct AirParam {
    pub id: LocalId,
    pub ty: AirType,
    pub name: String,
    pub span: Option<Span>,
}

#[derive(Clone)]
pub struct AirLocal {
    pub id: LocalId,
    pub ty: AirType,
    pub name: Option<String>,
    pub is_mut: bool,
    pub span: Option<Span>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GcMode {
    Managed,
    Manual,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallingConv {
    Aelys,
    C,
    Rust,
}

#[derive(Clone)]
pub struct AirBlock {
    pub id: BlockId,
    pub stmts: Vec<AirStmt>,
    pub terminator: AirTerminator,
}

#[derive(Clone)]
pub struct AirStmt {
    pub kind: AirStmtKind,
    pub span: Option<Span>,
}

#[derive(Clone)]
pub enum AirStmtKind {
    Assign {
        place: Place,
        rvalue: Rvalue,
    },
    GcAlloc {
        local: LocalId,
        ty: AirType,
        arena: ArenaId,
    },
    GcDrop(LocalId),
    ArenaCreate(ArenaId),
    ArenaDestroy(ArenaId),
    Alloc {
        local: LocalId,
        ty: AirType,
    },
    RcAlloc {
        local: LocalId,
        ty: AirType,
    },
    Free(LocalId),
    CallVoid {
        func: Callee,
        args: Vec<Operand>,
    },
    MemoryFence(Ordering),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ordering {
    Relaxed,
    Acquire,
    Release,
    AcqRel,
    SeqCst,
}

#[derive(Clone)]
pub enum Rvalue {
    Use(Operand),
    BinaryOp(BinOp, Operand, Operand),
    UnaryOp(UnOp, Operand),
    Call {
        func: Callee,
        args: Vec<Operand>,
    },
    StructInit {
        name: String,
        fields: Vec<(String, Operand)>,
    },
    FieldAccess {
        base: Operand,
        field: String,
    },
    AddressOf(Place),
    Deref(Operand),
    Cast {
        operand: Operand,
        from: AirType,
        to: AirType,
    },
    Index {
        base: Operand,
        index: Operand,
    },
    EnumInit {
        enum_ref: EnumRef,
        variant: String,
        tag: u32,
        payload: Vec<Operand>, // empty for unit variants
    },
    EnumTag {
        enum_ref: EnumRef,
        operand: Operand,
    },
    EnumPayload {
        enum_ref: EnumRef,
        tag: u32,
        operand: Operand,
        field_index: u32,
    },
    ClosureCreate {
        fn_name: String,
        env: Operand,
    },
    SliceFromParts {
        ptr: Operand,
        len: Operand,
    },
    /// the operand is a `ptr(array|slice|vec)` local, never the collection value itself
    Len(Operand),
}

#[derive(Clone)]
pub enum Callee {
    Direct(FunctionId),
    Named(String),
    FnPtr(LocalId),
    Extern(String, CallingConv),
}

#[derive(Clone)]
pub enum Operand {
    Copy(LocalId),
    Move(LocalId),
    Const(AirConst),
}

#[derive(Clone)]
pub enum AirConst {
    IntLiteral(i64),
    Int(i64, AirIntSize),
    Float(f64, AirFloatSize),
    Bool(bool),
    Str(String),
    Null,
    FnRef(String),
    Enum {
        enum_ref: EnumRef,
        tag: u32,
        payload: Vec<AirConst>,
    },
    ZeroInit(AirType),
    Undef(AirType),
    Array(Vec<AirConst>),
    Struct {
        name: String,
        fields: Vec<(String, AirConst)>,
    },
}

#[derive(Clone)]
pub enum Place {
    Local(LocalId),
    Global(String),
    Field(LocalId, String),
    Deref(LocalId),
    Index(LocalId, Operand),
}

#[derive(Clone)]
pub enum AirTerminator {
    Return(Option<Operand>),
    Goto(BlockId),
    Branch {
        cond: Operand,
        then_block: BlockId,
        else_block: BlockId,
    },
    Switch {
        discr: Operand,
        targets: Vec<(AirConst, BlockId)>,
        default: BlockId,
    },
    Invoke {
        func: Callee,
        args: Vec<Operand>,
        ret: Place,
        normal: BlockId,
        unwind: BlockId,
    },
    Unwind,
    Unreachable,
    Panic {
        message: String,
        span: Option<Span>,
    },
}

#[derive(Clone)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    And,
    Or,
    BitAnd,
    BitOr,
    BitXor,
    Shl,
    Shr,
    CheckedAdd,
    CheckedSub,
    CheckedMul,
}

#[derive(Clone)]
pub enum UnOp {
    Neg,
    Not,
    BitNot,
}

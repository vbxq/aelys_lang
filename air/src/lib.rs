// AIR, the Aelys Intermediate Representation

pub mod layout;
pub mod lower;
pub mod mono;

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
pub enum AirType {
    I8, I16, I32, I64,
    U8, U16, U32, U64,
    F32, F64,
    Bool,
    Str,
    Ptr(Box<AirType>),
    Struct(String),
    Array(Box<AirType>, u64),
    Slice(Box<AirType>),
    FnPtr {
        params: Vec<AirType>,
        ret: Box<AirType>,
        conv: CallingConv,
    },
    Param(TypeParamId),
    Void,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AirIntSize { I8, I16, I32, I64, U8, U16, U32, U64 }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AirFloatSize { F32, F64 }

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
pub struct AirProgram {
    pub functions: Vec<AirFunction>,
    pub structs: Vec<AirStructDef>,
    pub globals: Vec<AirGlobal>,
    pub source_files: Vec<String>,
    pub mono_instances: Vec<MonoInstance>,
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
    Free(LocalId),
    CallVoid {
        func: Callee,
        args: Vec<Operand>,
    },
    MemoryFence(Ordering),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ordering {
    Relaxed, Acquire, Release, AcqRel, SeqCst,
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
    AddressOf(LocalId),
    Deref(Operand),
    Cast {
        operand: Operand,
        from: AirType,
        to: AirType,
    },
    Discriminant(Operand),
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
    ZeroInit(AirType),
    Undef(AirType),
}

#[derive(Clone)]
pub enum Place {
    Local(LocalId),
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
    Add, Sub, Mul, Div, Rem,
    Eq, Ne, Lt, Le, Gt, Ge,
    And, Or,
    BitAnd, BitOr, BitXor, Shl, Shr,
    CheckedAdd, CheckedSub, CheckedMul,
}

#[derive(Clone)]
pub enum UnOp {
    Neg, Not, BitNot,
}

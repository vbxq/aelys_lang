
use std::sync::Arc;

use aelys_syntax::Source;
use aelys_syntax::Span;
use aelys_syntax::{BinaryOp, Decorator, NeedsStmt, UnaryOp};

use crate::types::InferType;
use crate::types::TypeTable;

#[derive(Debug, Clone)]
pub struct TypedProgram {
    pub stmts: Vec<TypedStmt>,
    pub source: Arc<Source>,
    pub type_table: TypeTable,
}

#[derive(Debug, Clone)]
pub struct TypedStmt {
    pub kind: TypedStmtKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypedStmtKind {
    Expression(TypedExpr),

    Let {
        name: String,
        mutable: bool,
        initializer: TypedExpr,
        var_type: InferType,
        is_pub: bool,
    },

    Block(Vec<TypedStmt>),

    If {
        condition: TypedExpr,
        then_branch: Box<TypedStmt>,
        else_branch: Option<Box<TypedStmt>>,
    },

    While {
        condition: TypedExpr,
        body: Box<TypedStmt>,
    },

    For {
        iterator: String,
        start: TypedExpr,
        end: TypedExpr,
        inclusive: bool,
        step: Box<Option<TypedExpr>>,
        body: Box<TypedStmt>,
    },

    ForEach {
        iterator: String,
        iterable: TypedExpr,
        elem_type: InferType,
        body: Box<TypedStmt>,
    },

    Return(Option<TypedExpr>),

    Break,
    Continue,

    Function(TypedFunction),

    Needs(NeedsStmt),

    StructDecl {
        name: String,
        type_params: Vec<String>,
        fields: Vec<(String, InferType)>,
        is_pub: bool,
    },

    EnumDecl {
        name: String,
        type_params: Vec<String>,
        variants: Vec<(String, u32, Vec<InferType>)>, // (variant_name, tag, data_types)
        is_pub: bool,
    },
}

#[derive(Debug, Clone)]
pub struct TypedFunction {
    pub name: String,
    pub type_params: Vec<String>,
    pub params: Vec<TypedParam>,
    pub return_type: InferType,
    pub body: Vec<TypedStmt>,
    pub decorators: Vec<Decorator>,
    pub is_pub: bool,
    pub declared_nogc: bool,
    pub span: Span,
    pub captures: Vec<(String, InferType)>,
}

#[derive(Debug, Clone)]
pub struct TypedParam {
    pub name: String,
    pub mutable: bool,
    pub ty: InferType,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub ty: InferType,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypedFmtStringPart {
    Literal(String),
    Expr(Box<TypedExpr>),
    Placeholder,
}

#[derive(Debug, Clone)]
pub enum TypedExprKind {
    Int(i64),
    Float(f64),
    Bool(bool),
    String(String),
    FmtString(Vec<TypedFmtStringPart>),
    Null,

    Identifier(String),

    Binary {
        left: Box<TypedExpr>,
        op: BinaryOp,
        right: Box<TypedExpr>,
    },

    Unary {
        op: UnaryOp,
        operand: Box<TypedExpr>,
    },

    And {
        left: Box<TypedExpr>,
        right: Box<TypedExpr>,
    },

    Or {
        left: Box<TypedExpr>,
        right: Box<TypedExpr>,
    },

    Call {
        callee: Box<TypedExpr>,
        args: Vec<TypedExpr>,
    },

    Assign {
        name: String,
        value: Box<TypedExpr>,
    },

    Grouping(Box<TypedExpr>),

    If {
        condition: Box<TypedExpr>,
        then_branch: Box<TypedExpr>,
        else_branch: Box<TypedExpr>,
    },

    Lambda(Box<TypedExpr>),

    LambdaInner {
        params: Vec<TypedParam>,
        return_type: InferType,
        body: Vec<TypedStmt>, // Changed to support multi-statement bodies
        captures: Vec<(String, InferType)>, // NEW: captured variables
    },

    Member {
        object: Box<TypedExpr>,
        member: String,
    },

    ArrayLiteral {
        elements: Vec<TypedExpr>,
    },

    ArraySized {
        size: Box<TypedExpr>,
        fill_value: Option<Box<TypedExpr>>,
    },

    VecLiteral {
        element_type: Option<crate::types::ResolvedType>,
        elements: Vec<TypedExpr>,
    },

    Index {
        object: Box<TypedExpr>,
        index: Box<TypedExpr>,
    },

    IndexAssign {
        object: Box<TypedExpr>,
        index: Box<TypedExpr>,
        value: Box<TypedExpr>,
    },

    FieldAssign {
        object: Box<TypedExpr>,
        field: String,
        value: Box<TypedExpr>,
    },

    Range {
        start: Option<Box<TypedExpr>>,
        end: Option<Box<TypedExpr>>,
        inclusive: bool,
    },

    Slice {
        object: Box<TypedExpr>,
        range: Box<TypedExpr>,
    },

    Reference {
        mutable: bool,
        operand: Box<TypedExpr>,
    },
    Deref(Box<TypedExpr>),
    DerefAssign {
        target: Box<TypedExpr>,
        value: Box<TypedExpr>,
    },

    StructLiteral {
        name: String,
        fields: Vec<(String, Box<TypedExpr>)>,
    },

    Cast {
        expr: Box<TypedExpr>,
        target: InferType,
    },

    EnumVariant {
        enum_name: String,
        variant: String,
        tag: u32,
        args: Vec<TypedExpr>, // empty for unit variants
    },

    Match {
        scrutinee: Box<TypedExpr>,
        arms: Vec<TypedMatchArm>,
    },

    ResultAssert {
        scrutinee: Box<TypedExpr>,
        ok_tag: u32,
        payload_ty: InferType,
        on_err: ResultAssertOnErr,
    },

    Block {
        stmts: Vec<TypedStmt>,
        tail: Box<TypedExpr>,
    },
}

#[derive(Debug, Clone)]
pub enum ResultAssertOnErr {
    Panic(String),
    Unreachable,
}

#[derive(Debug, Clone)]
pub struct TypedMatchArm {
    pub pattern: TypedPattern,
    pub body: Box<TypedExpr>,
}

#[derive(Debug, Clone)]
pub enum TypedPattern {
    Variant {
        enum_name: String,
        variant: String,
        tag: u32,
        bindings: Vec<(String, InferType)>,
    },
    Wildcard,
}

impl TypedExpr {
    pub fn new(kind: TypedExprKind, ty: InferType, span: Span) -> Self {
        Self { kind, ty, span }
    }

    pub fn has_concrete_type(&self) -> bool {
        !matches!(self.ty, InferType::Var(_) | InferType::Dynamic)
    }
}

impl TypedStmt {
    pub fn new(kind: TypedStmtKind, span: Span) -> Self {
        Self { kind, span }
    }
}

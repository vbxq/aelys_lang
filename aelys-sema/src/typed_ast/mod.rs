// AST with type annotations for codegen

use std::sync::Arc;

use aelys_syntax::Source;
use aelys_syntax::Span;
use aelys_syntax::{BinaryOp, Decorator, NeedsStmt, UnaryOp};

use crate::types::InferType;

/// A fully typed program
#[derive(Debug, Clone)]
pub struct TypedProgram {
    pub stmts: Vec<TypedStmt>,
    pub source: Arc<Source>,
}

/// A typed statement
#[derive(Debug, Clone)]
pub struct TypedStmt {
    pub kind: TypedStmtKind,
    pub span: Span,
}

/// Typed statement kinds
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
        step: Option<TypedExpr>,
        body: Box<TypedStmt>,
    },

    Return(Option<TypedExpr>),

    Break,
    Continue,

    Function(TypedFunction),

    Needs(NeedsStmt),
}

/// A typed function
#[derive(Debug, Clone)]
pub struct TypedFunction {
    pub name: String,
    pub params: Vec<TypedParam>,
    pub return_type: InferType,
    pub body: Vec<TypedStmt>,
    pub decorators: Vec<Decorator>,
    pub is_pub: bool,
    pub span: Span,
    /// Captured variables from enclosing scopes (for closures)
    pub captures: Vec<(String, InferType)>,
}

/// A typed parameter
#[derive(Debug, Clone)]
pub struct TypedParam {
    pub name: String,
    pub ty: InferType,
    pub span: Span,
}

/// A typed expression
#[derive(Debug, Clone)]
pub struct TypedExpr {
    pub kind: TypedExprKind,
    pub ty: InferType,
    pub span: Span,
}

/// Part of a typed format string
#[derive(Debug, Clone)]
pub enum TypedFmtStringPart {
    Literal(String),
    Expr(Box<TypedExpr>),
    Placeholder,
}

/// Typed expression kinds
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

    /// Inner lambda structure (params + body + captures)
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
        element_type: Option<crate::types::ResolvedType>,
        elements: Vec<TypedExpr>,
    },

    ArraySized {
        element_type: Option<crate::types::ResolvedType>,
        size: Box<TypedExpr>,
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

    Range {
        start: Option<Box<TypedExpr>>,
        end: Option<Box<TypedExpr>>,
        inclusive: bool,
    },

    Slice {
        object: Box<TypedExpr>,
        range: Box<TypedExpr>,
    },
}

impl TypedExpr {
    /// Create a new typed expression
    pub fn new(kind: TypedExprKind, ty: InferType, span: Span) -> Self {
        Self { kind, ty, span }
    }

    /// Check if this expression has a known concrete type
    pub fn has_concrete_type(&self) -> bool {
        !matches!(self.ty, InferType::Var(_) | InferType::Dynamic)
    }
}

impl TypedStmt {
    pub fn new(kind: TypedStmtKind, span: Span) -> Self {
        Self { kind, span }
    }
}

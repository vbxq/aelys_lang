use super::expr::{Expr, Parameter, TypeAnnotation};
use crate::Span;

#[derive(Debug, Clone)]
pub struct Stmt {
    pub kind: StmtKind,
    pub span: Span,
}

impl Stmt {
    pub fn new(kind: StmtKind, span: Span) -> Self {
        Self { kind, span }
    }
}

#[derive(Debug, Clone)]
pub enum StmtKind {
    Expression(Expr),

    Let {
        name: String,
        mutable: bool,
        type_annotation: Option<TypeAnnotation>,
        initializer: Expr,
        is_pub: bool,
    },

    Block(Vec<Stmt>),

    If {
        condition: Expr,
        then_branch: Box<Stmt>,
        else_branch: Option<Box<Stmt>>,
    },
    While {
        condition: Expr,
        body: Box<Stmt>,
    },

    // for i in start..end { } or start..=end (inclusive)
    For {
        iterator: String,
        start: Expr,
        end: Expr,
        inclusive: bool,
        step: Box<Option<Expr>>, // default: inferred from direction
        body: Box<Stmt>,
    },

    // for item in collection { }
    ForEach {
        iterator: String,
        iterable: Expr,
        body: Box<Stmt>,
    },

    Break,
    Continue,
    Return(Option<Expr>),
    Function(Function),
    Needs(NeedsStmt),

    StructDecl {
        name: String,
        fields: Vec<StructFieldDecl>,
        is_pub: bool,
    },
}

#[derive(Debug, Clone)]
pub struct StructFieldDecl {
    pub name: String,
    pub type_annotation: TypeAnnotation,
    pub span: Span,
}

// module import - `needs utils.helpers` or `needs cos, sin from std.math`
#[derive(Debug, Clone)]
pub struct NeedsStmt {
    pub path: Vec<String>, // ["utils", "helpers"]
    pub kind: ImportKind,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum ImportKind {
    Module { alias: Option<String> }, // needs foo.bar (as alias)?
    Symbols(Vec<String>),             // needs x, y from foo.bar
    Wildcard,                         // needs foo.bar.*
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub params: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub body: Vec<Stmt>,
    pub decorators: Vec<Decorator>,
    pub is_pub: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Decorator {
    pub name: String,
    pub span: Span,
}

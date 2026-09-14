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

    Discard(Expr),

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

    For {
        iterator: String,
        start: Expr,
        end: Expr,
        inclusive: bool,
        step: Box<Option<Expr>>, // default: inferred from direction
        body: Box<Stmt>,
    },

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
        type_params: Vec<String>,
        fields: Vec<StructFieldDecl>,
        is_pub: bool,
    },

    EnumDecl {
        name: String,
        type_params: Vec<String>,
        variants: Vec<EnumVariantDecl>,
        is_pub: bool,
    },
}

#[derive(Debug, Clone)]
pub struct StructFieldDecl {
    pub name: String,
    pub type_annotation: TypeAnnotation,
    pub is_pub: bool,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct EnumVariantDecl {
    pub name: String,
    pub fields: Vec<TypeAnnotation>, // empty = unit variant, non-empty = tuple variant
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct NeedsStmt {
    pub target: NeedsTarget,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum NeedsTarget {
    Module { path: Vec<String>, kind: ImportKind },
    Foreign { header: String },
}

#[derive(Debug, Clone)]
pub enum ImportKind {
    Module { alias: Option<String> }, // needs foo.bar (as alias)?
    Symbols(Vec<String>),             // needs x, y from foo.bar
    Wildcard,                         // needs foo.bar.*
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForeignConv {
    C,
}

#[derive(Debug, Clone)]
pub struct ForeignDecl {
    pub symbol: String,
    pub calling_conv: ForeignConv,
    pub is_unsafe: bool,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TypeBounds {
    pub nogc: bool,
    pub eq: bool,
    pub ord: bool,
}

impl TypeBounds {
    pub const NAMES: [&'static str; 3] = ["nogc", "eq", "ord"];

    pub fn set(&mut self, name: &str) -> bool {
        match name {
            "nogc" => self.nogc = true,
            "eq" => self.eq = true,
            "ord" => self.ord = true,
            _ => return false,
        }
        true
    }

    pub fn is_empty(&self) -> bool {
        !self.nogc && !self.eq && !self.ord
    }

    pub fn needs_eq(&self) -> bool {
        self.eq || self.ord
    }

    pub fn needs_ord(&self) -> bool {
        self.ord
    }

    pub fn spelling(&self) -> String {
        let mut parts: Vec<&str> = Vec::new();
        if self.nogc {
            parts.push("nogc");
        }
        if self.eq {
            parts.push("eq");
        }
        if self.ord {
            parts.push("ord");
        }
        parts.join(" + ")
    }
}

#[derive(Debug, Clone)]
pub struct Function {
    pub name: String,
    pub type_params: Vec<String>,
    pub bounds: Vec<TypeBounds>,
    pub params: Vec<Parameter>,
    pub return_type: Option<TypeAnnotation>,
    pub body: Vec<Stmt>,
    pub decorators: Vec<Decorator>,
    pub is_pub: bool,
    pub is_nogc: bool,
    pub foreign: Option<ForeignDecl>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub struct Decorator {
    pub name: String,
    pub span: Span,
}

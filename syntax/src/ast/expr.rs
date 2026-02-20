use crate::Span;

#[derive(Debug, Clone)]
pub struct TypeAnnotation {
    pub name: String, // "int", "float", "string", "array", "vec", etc
    pub type_param: Option<Box<TypeAnnotation>>, // for generics: array<int>, vec<string>
    pub span: Span,
}

impl TypeAnnotation {
    pub fn new(name: String, span: Span) -> Self {
        Self {
            name,
            type_param: None,
            span,
        }
    }

    pub fn with_param(name: String, type_param: TypeAnnotation, span: Span) -> Self {
        Self {
            name,
            type_param: Some(Box::new(type_param)),
            span,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Parameter {
    pub name: String,
    pub mutable: bool,
    pub type_annotation: Option<TypeAnnotation>, // None = inferred
    pub span: Span,
}

impl Parameter {
    pub fn new(
        name: String,
        mutable: bool,
        type_annotation: Option<TypeAnnotation>,
        span: Span,
    ) -> Self {
        Self {
            name,
            mutable,
            type_annotation,
            span,
        }
    }

    pub fn untyped(name: String, span: Span) -> Self {
        Self {
            name,
            mutable: false,
            type_annotation: None,
            span,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Expr {
    pub kind: ExprKind,
    pub span: Span,
}

impl Expr {
    pub fn new(kind: ExprKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Part of a format string in the AST (after parsing expressions)
#[derive(Debug, Clone)]
pub enum FmtStringPart {
    Literal(String),
    Expr(Box<Expr>),
    Placeholder,
}

#[derive(Debug, Clone)]
pub enum ExprKind {
    // literals
    Int(i64),
    Float(f64),
    String(String),
    Bool(bool),
    Null,
    FmtString(Vec<FmtStringPart>),

    Identifier(String),

    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        operand: Box<Expr>,
    },

    // short-circuit (separate from Binary because different codegen)
    And {
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Or {
        left: Box<Expr>,
        right: Box<Expr>,
    },

    Call {
        callee: Box<Expr>,
        args: Vec<Expr>,
    },
    Assign {
        name: String,
        value: Box<Expr>,
    },
    Grouping(Box<Expr>), // for precedence

    // ternary: cond ? then : else
    If {
        condition: Box<Expr>,
        then_branch: Box<Expr>,
        else_branch: Box<Expr>,
    },

    Lambda {
        params: Vec<Parameter>,
        return_type: Option<TypeAnnotation>,
        body: Vec<crate::ast::Stmt>,
    },

    Member {
        object: Box<Expr>,
        member: String,
    }, // module.symbol

    // Arrays and Vecs
    ArrayLiteral {
        element_type: Option<TypeAnnotation>, // Array<Int>[...] or Array[...]
        elements: Vec<Expr>,
    },
    ArraySized {
        element_type: Option<TypeAnnotation>, // Array<int>(10) or Array(10) or [; 10]
        size: Box<Expr>,
    },
    VecLiteral {
        element_type: Option<TypeAnnotation>,
        elements: Vec<Expr>,
    },
    Index {
        object: Box<Expr>,
        index: Box<Expr>,
    },
    IndexAssign {
        object: Box<Expr>,
        index: Box<Expr>,
        value: Box<Expr>,
    },
    Range {
        start: Option<Box<Expr>>,
        end: Option<Box<Expr>>,
        inclusive: bool, // .. vs ..=
    },
    Slice {
        object: Box<Expr>,
        range: Box<Expr>,
    },

    StructLiteral {
        name: String,
        fields: Vec<StructFieldInit>,
    },

    Cast {
        expr: Box<Expr>,
        target: TypeAnnotation,
    },
}

#[derive(Debug, Clone)]
pub struct StructFieldInit {
    pub name: String,
    pub value: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
    Shl,
    Shr,
    BitAnd,
    BitOr,
    BitXor,
}

impl BinaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
            Self::Eq => "==",
            Self::Ne => "!=",
            Self::Lt => "<",
            Self::Le => "<=",
            Self::Gt => ">",
            Self::Ge => ">=",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::BitAnd => "&",
            Self::BitOr => "|",
            Self::BitXor => "^",
        }
    }
}

impl std::fmt::Display for BinaryOp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Neg,    // -
    Not,    // not
    BitNot, // ~
}

impl UnaryOp {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Neg => "-",
            Self::Not => "not",
            Self::BitNot => "~",
        }
    }
}

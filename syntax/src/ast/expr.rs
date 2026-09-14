use crate::Span;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RefKind {
    Shared,
    Mut,
}

#[derive(Debug, Clone)]
pub struct TypeAnnotation {
    pub name: String,
    pub type_param: Option<Box<TypeAnnotation>>,
    /// multiple type parameters for generic types like `result<i64, string>`.
    pub type_params: Vec<TypeAnnotation>,
    pub fn_params: Option<Vec<TypeAnnotation>>,
    pub fn_ret: Option<Box<TypeAnnotation>>,
    pub array_size: Option<u64>,
    pub reference: Option<RefKind>,
    pub is_slice: bool,
    pub nogc: bool,
    pub span: Span,
}

impl TypeAnnotation {
    pub fn new(name: String, span: Span) -> Self {
        Self {
            name,
            type_param: None,
            type_params: Vec::new(),
            fn_params: None,
            fn_ret: None,
            array_size: None,
            reference: None,
            is_slice: false,
            nogc: false,
            span,
        }
    }

    pub fn with_param(name: String, type_param: TypeAnnotation, span: Span) -> Self {
        Self {
            name,
            type_param: Some(Box::new(type_param)),
            type_params: Vec::new(),
            fn_params: None,
            fn_ret: None,
            array_size: None,
            reference: None,
            is_slice: false,
            nogc: false,
            span,
        }
    }

    pub fn slice_referent(element: TypeAnnotation, span: Span) -> Self {
        Self {
            name: "[slice]".to_string(),
            type_param: Some(Box::new(element)),
            type_params: Vec::new(),
            fn_params: None,
            fn_ret: None,
            array_size: None,
            reference: None,
            is_slice: true,
            nogc: false,
            span,
        }
    }

    pub fn with_params(name: String, type_params: Vec<TypeAnnotation>, span: Span) -> Self {
        let single = if type_params.len() == 1 {
            Some(Box::new(type_params[0].clone()))
        } else {
            None
        };
        Self {
            name,
            type_param: single,
            type_params,
            fn_params: None,
            fn_ret: None,
            array_size: None,
            reference: None,
            is_slice: false,
            nogc: false,
            span,
        }
    }

    pub fn function_type(
        params: Vec<TypeAnnotation>,
        ret: TypeAnnotation,
        nogc: bool,
        span: Span,
    ) -> Self {
        Self {
            name: "fn".to_string(),
            type_param: None,
            type_params: Vec::new(),
            fn_params: Some(params),
            fn_ret: Some(Box::new(ret)),
            array_size: None,
            reference: None,
            is_slice: false,
            nogc,
            span,
        }
    }

    pub fn array_sized(inner: TypeAnnotation, size: u64, span: Span) -> Self {
        Self {
            name: "array".to_string(),
            type_param: Some(Box::new(inner)),
            type_params: Vec::new(),
            fn_params: None,
            fn_ret: None,
            array_size: Some(size),
            reference: None,
            is_slice: false,
            nogc: false,
            span,
        }
    }

    pub fn is_function_type(&self) -> bool {
        self.fn_params.is_some()
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
    Char(u32),
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

    ArrayLiteral {
        elements: Vec<Expr>,
    },
    ArraySized {
        size: Box<Expr>,
        fill_value: Option<Box<Expr>>, // [val; N] syntax
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
    FieldAssign {
        object: Box<Expr>,
        field: String,
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

    Reference {
        mutable: bool,
        operand: Box<Expr>,
    },
    Deref(Box<Expr>),
    DerefAssign {
        target: Box<Expr>,
        value: Box<Expr>,
    },

    StructLiteral {
        name: String,
        fields: Vec<StructFieldInit>,
    },

    EnumVariant {
        enum_name: String,
        variant: String,
        args: Vec<Expr>, // empty for unit variants
    },

    Cast {
        expr: Box<Expr>,
        target: TypeAnnotation,
    },

    Match {
        scrutinee: Box<Expr>,
        arms: Vec<MatchArm>,
    },

    Block {
        stmts: Vec<crate::ast::Stmt>,
        tail: Box<Expr>,
    },

    // postfix ?; desugared away in sema, has no typed-ast counterpart
    Try(Box<Expr>),

    // postfix catch; desugared to a match in sema, has no typed-ast counterpart
    Catch {
        scrutinee: Box<Expr>,
        handler: CatchHandler,
    },

    Unsafe(Box<Expr>),
}

#[derive(Debug, Clone)]
pub enum CatchHandler {
    Binding { name: String, body: Box<Expr> },
    Arms(Vec<MatchArm>),
}

#[derive(Debug, Clone)]
pub struct MatchArm {
    pub pattern: Pattern,
    pub body: Box<Expr>,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum Pattern {
    Variant {
        enum_name: String,
        variant: String,
        bindings: Vec<String>,
        span: Span,
    },
    Wildcard(Span),
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

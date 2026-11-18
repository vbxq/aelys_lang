
use std::sync::Arc;

use aelys_syntax::Source;
use aelys_syntax::Span;
use aelys_syntax::{BinaryOp, Decorator, ForeignDecl, NeedsStmt, TypeBounds, UnaryOp};
pub use aelys_syntax::{MatchOrigin, PatLit};

use crate::types::InferType;
use crate::types::TypeTable;

#[derive(Debug, Clone)]
pub struct TypedProgram {
    pub stmts: Vec<TypedStmt>,
    pub source: Arc<Source>,
    pub type_table: TypeTable,
    pub literal_globals: std::collections::HashMap<String, LiteralInit>,
    pub decls: ModuleDecls,
    pub traits: Vec<crate::modules::TraitDef>,
    pub impls: Vec<crate::modules::ImplDef>,
}

#[derive(Debug, Clone, Default)]
pub struct ModuleDecls {
    pub consts: Vec<(String, bool, ConstLit)>,
    pub aliases: Vec<(String, crate::modules::AliasDef)>,
    pub foreign_methods: Vec<(String, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LiteralInit {
    Int(i64),
    Float(f64),
    Const(ConstLit),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConstLit {
    pub ty: InferType,
    pub value: ConstScalar,
}

#[derive(Debug, Clone)]
pub enum ConstScalar {
    Int(i64),
    Float(f64),
    Bool(bool),
    Char(u32),
    Str(String),
}

impl PartialEq for ConstScalar {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            // a nan const has to equal itself or a module cycle holding one never settles
            (ConstScalar::Float(a), ConstScalar::Float(b)) => a.to_bits() == b.to_bits(),
            (ConstScalar::Int(a), ConstScalar::Int(b)) => a == b,
            (ConstScalar::Bool(a), ConstScalar::Bool(b)) => a == b,
            (ConstScalar::Char(a), ConstScalar::Char(b)) => a == b,
            (ConstScalar::Str(a), ConstScalar::Str(b)) => a == b,
            _ => false,
        }
    }
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

    Break(Option<TypedExpr>),
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
    pub bounds: Vec<TypeBounds>,
    pub params: Vec<TypedParam>,
    pub return_type: InferType,
    pub body: Vec<TypedStmt>,
    pub decorators: Vec<Decorator>,
    pub is_pub: bool,
    pub declared_nogc: bool,
    pub foreign: Option<ForeignDecl>,
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
    Char(u32),
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
        receiver_of: Option<String>,
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
        origin: MatchOrigin,
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

    // the statement is the `while true` loop whose `break value` statements give the value
    Loop(Box<TypedStmt>),
}

#[derive(Debug, Clone)]
pub enum ResultAssertOnErr {
    Panic(String),
    Unreachable,
}

#[derive(Debug, Clone)]
pub struct TypedMatchArm {
    pub pattern: TypedPattern,
    guard: Option<Box<TypedExpr>>,
    rhs: Box<TypedExpr>,
}

impl TypedMatchArm {
    pub fn new(pattern: TypedPattern, guard: Option<TypedExpr>, body: TypedExpr) -> Self {
        Self {
            pattern,
            guard: guard.map(Box::new),
            rhs: Box::new(body),
        }
    }

    pub fn guard(&self) -> Option<&TypedExpr> {
        self.guard.as_deref()
    }

    pub fn body(&self) -> &TypedExpr {
        &self.rhs
    }

    pub fn body_mut(&mut self) -> &mut TypedExpr {
        &mut self.rhs
    }

    pub fn body_box_mut(&mut self) -> &mut Box<TypedExpr> {
        &mut self.rhs
    }

    pub fn exprs(&self) -> impl Iterator<Item = &TypedExpr> {
        self.guard
            .as_deref()
            .into_iter()
            .chain(std::iter::once(&*self.rhs))
    }

    pub fn exprs_mut(&mut self) -> impl Iterator<Item = &mut TypedExpr> {
        self.guard
            .as_deref_mut()
            .into_iter()
            .chain(std::iter::once(&mut *self.rhs))
    }

    pub fn bindings(&self) -> Vec<(&str, &InferType, Span)> {
        self.pattern.bindings()
    }

    pub fn map_exprs(self, mut f: impl FnMut(TypedExpr) -> TypedExpr) -> Self {
        Self {
            pattern: self.pattern,
            guard: self.guard.map(|g| Box::new(f(*g))),
            rhs: Box::new(f(*self.rhs)),
        }
    }
}

#[derive(Debug, Clone)]
pub struct TypedPattern {
    pub kind: TypedPatternKind,
    pub ty: InferType,
    pub span: Span,
}

#[derive(Debug, Clone)]
pub enum TypedPatternKind {
    Wildcard,
    Binding {
        name: String,
        sub: Option<Box<TypedPattern>>,
    },
    Lit(PatLit),
    Range {
        lo: PatLit,
        hi: PatLit,
    },
    Variant {
        enum_name: String,
        variant: String,
        tag: u32,
        fields: Vec<TypedPattern>,
    },
    Struct {
        name: String,
        fields: Vec<(String, TypedPattern)>,
    },
    Tuple(Vec<TypedPattern>),
    Or(Vec<TypedPattern>),
}

impl TypedPattern {
    pub fn new(kind: TypedPatternKind, ty: InferType, span: Span) -> Self {
        Self { kind, ty, span }
    }

    pub fn bindings(&self) -> Vec<(&str, &InferType, Span)> {
        let mut out = Vec::new();
        self.collect_bindings(&mut out);
        out
    }

    fn collect_bindings<'a>(&'a self, out: &mut Vec<(&'a str, &'a InferType, Span)>) {
        match &self.kind {
            TypedPatternKind::Wildcard
            | TypedPatternKind::Lit(_)
            | TypedPatternKind::Range { .. } => {}
            TypedPatternKind::Binding { name, sub } => {
                out.push((name.as_str(), &self.ty, self.span));
                if let Some(sub) = sub {
                    sub.collect_bindings(out);
                }
            }
            TypedPatternKind::Variant { fields, .. } | TypedPatternKind::Tuple(fields) => {
                for f in fields {
                    f.collect_bindings(out);
                }
            }
            TypedPatternKind::Struct { fields, .. } => {
                for (_, f) in fields {
                    f.collect_bindings(out);
                }
            }
            TypedPatternKind::Or(alts) => {
                if let Some(first) = alts.first() {
                    first.collect_bindings(out);
                }
            }
        }
    }

    pub fn irrefutable_shallow(&self) -> bool {
        match &self.kind {
            TypedPatternKind::Wildcard => true,
            TypedPatternKind::Binding { sub, .. } => {
                sub.as_ref().is_none_or(|s| s.irrefutable_shallow())
            }
            _ => false,
        }
    }

    pub fn map_types(self, f: &mut impl FnMut(InferType) -> InferType) -> Self {
        let kind = match self.kind {
            TypedPatternKind::Binding { name, sub } => TypedPatternKind::Binding {
                name,
                sub: sub.map(|s| Box::new(s.map_types(f))),
            },
            TypedPatternKind::Variant {
                enum_name,
                variant,
                tag,
                fields,
            } => TypedPatternKind::Variant {
                enum_name,
                variant,
                tag,
                fields: fields.into_iter().map(|p| p.map_types(f)).collect(),
            },
            TypedPatternKind::Struct { name, fields } => TypedPatternKind::Struct {
                name,
                fields: fields
                    .into_iter()
                    .map(|(n, p)| (n, p.map_types(f)))
                    .collect(),
            },
            TypedPatternKind::Tuple(elems) => {
                TypedPatternKind::Tuple(elems.into_iter().map(|p| p.map_types(f)).collect())
            }
            TypedPatternKind::Or(alts) => {
                TypedPatternKind::Or(alts.into_iter().map(|p| p.map_types(f)).collect())
            }
            other => other,
        };
        Self {
            kind,
            ty: f(self.ty),
            span: self.span,
        }
    }
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

use crate::Span;

#[derive(Debug, Clone, PartialEq)]
pub struct Token {
    pub kind: TokenKind,
    pub span: Span,
}

impl Token {
    pub fn new(kind: TokenKind, span: Span) -> Self {
        Self { kind, span }
    }
}

/// Part of a format string: either literal text, a placeholder {}, or an expression {expr}
#[derive(Debug, Clone, PartialEq)]
pub enum FmtPart {
    Literal(String),
    Placeholder,
    Expr(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    // literals
    Int(i64),
    Float(f64),
    String(String),
    FmtString(Vec<FmtPart>),
    True,
    False,
    Null,

    Identifier(String),

    // keywords
    Let,
    Mut,
    Fn,
    If,
    Else,
    While,
    Return,
    Break,
    Continue,
    And,
    Or,
    Not,
    Pub,
    Needs,
    As,
    From, // module system
    For,
    In,
    Step,
    Struct,

    // operators
    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Eq,
    EqEq,
    BangEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    Arrow,      // ->
    Colon,      // :
    PlusEq,     // +=
    MinusEq,    // -=
    StarEq,     // *=
    SlashEq,    // /=
    PercentEq,  // %=
    PlusPlus,   // ++
    MinusMinus, // --

    // bitwise
    Shl,
    Shr,       // << >>
    Ampersand, // &
    Pipe,
    Caret, // | ^
    Tilde, // ~

    // delimiters
    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,
    Comma,
    Semicolon,
    Dot,      // .
    DotDot,   // ..
    DotDotEq, // ..=

    // special
    At,      // @ for decorators
    Newline, // for auto-semicolon insertion
    Eof,
}

impl TokenKind {
    pub fn is_literal(&self) -> bool {
        matches!(
            self,
            Self::Int(_)
                | Self::Float(_)
                | Self::String(_)
                | Self::FmtString(_)
                | Self::True
                | Self::False
                | Self::Null
        )
    }

    // semicolon insertion (Go-style, roughly)
    pub fn can_end_statement(&self) -> bool {
        matches!(
            self,
            Self::Identifier(_)
                | Self::Int(_)
                | Self::Float(_)
                | Self::String(_)
                | Self::FmtString(_)
                | Self::True
                | Self::False
                | Self::Null
                | Self::Break
                | Self::Continue
                | Self::Return
                | Self::RParen
                | Self::RBracket
                | Self::RBrace
                | Self::Star // for `needs module.*`
                | Self::PlusPlus
                | Self::MinusMinus
        )
    }
}

impl std::fmt::Display for TokenKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Int(n) => write!(f, "{}", n),
            Self::Float(n) => write!(f, "{}", n),
            Self::String(s) => write!(f, "\"{}\"", s),
            Self::FmtString(_) => write!(f, "<format string>"),
            Self::True => write!(f, "true"),
            Self::False => write!(f, "false"),
            Self::Null => write!(f, "null"),
            Self::Identifier(s) => write!(f, "{}", s),
            Self::Let => write!(f, "let"),
            Self::Mut => write!(f, "mut"),
            Self::Fn => write!(f, "fn"),
            Self::If => write!(f, "if"),
            Self::Else => write!(f, "else"),
            Self::While => write!(f, "while"),
            Self::Return => write!(f, "return"),
            Self::Break => write!(f, "break"),
            Self::Continue => write!(f, "continue"),
            Self::And => write!(f, "and"),
            Self::Or => write!(f, "or"),
            Self::Not => write!(f, "not"),
            Self::Pub => write!(f, "pub"),
            Self::Needs => write!(f, "needs"),
            Self::As => write!(f, "as"),
            Self::From => write!(f, "from"),
            Self::For => write!(f, "for"),
            Self::In => write!(f, "in"),
            Self::Step => write!(f, "step"),
            Self::Struct => write!(f, "struct"),
            Self::Plus => write!(f, "+"),
            Self::Minus => write!(f, "-"),
            Self::Star => write!(f, "*"),
            Self::Slash => write!(f, "/"),
            Self::Percent => write!(f, "%"),
            Self::Eq => write!(f, "="),
            Self::EqEq => write!(f, "=="),
            Self::BangEq => write!(f, "!="),
            Self::Lt => write!(f, "<"),
            Self::LtEq => write!(f, "<="),
            Self::Gt => write!(f, ">"),
            Self::GtEq => write!(f, ">="),
            Self::Arrow => write!(f, "->"),
            Self::Colon => write!(f, ":"),
            Self::PlusEq => write!(f, "+="),
            Self::MinusEq => write!(f, "-="),
            Self::StarEq => write!(f, "*="),
            Self::SlashEq => write!(f, "/="),
            Self::PercentEq => write!(f, "%="),
            Self::PlusPlus => write!(f, "++"),
            Self::MinusMinus => write!(f, "--"),
            Self::Shl => write!(f, "<<"),
            Self::Shr => write!(f, ">>"),
            Self::Ampersand => write!(f, "&"),
            Self::Pipe => write!(f, "|"),
            Self::Caret => write!(f, "^"),
            Self::Tilde => write!(f, "~"),
            Self::LParen => write!(f, "("),
            Self::RParen => write!(f, ")"),
            Self::LBrace => write!(f, "{{"),
            Self::RBrace => write!(f, "}}"),
            Self::LBracket => write!(f, "["),
            Self::RBracket => write!(f, "]"),
            Self::Comma => write!(f, ","),
            Self::Semicolon => write!(f, ";"),
            Self::Dot => write!(f, "."),
            Self::DotDot => write!(f, ".."),
            Self::DotDotEq => write!(f, "..="),
            Self::At => write!(f, "@"),
            Self::Newline => write!(f, "<newline>"),
            Self::Eof => write!(f, "<eof>"),
        }
    }
}

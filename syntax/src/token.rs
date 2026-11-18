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

#[derive(Debug, Clone, PartialEq)]
pub enum TokenKind {
    Int(i64),
    UInt(u64),
    Float(f64),
    SuffixedInt(u128, String),
    SuffixedFloat(f64, String),
    String(String),
    Char(u32),
    Byte(u8),
    Lifetime(String),
    True,
    False,

    Identifier(String),

    As,
    Break,
    Const,
    Continue,
    Crate,
    Dyn,
    Else,
    Enum,
    Extern,
    Fn,
    For,
    If,
    Impl,
    In,
    Let,
    Loop,
    Match,
    Mod,
    Move,
    Mut,
    Nogc,
    Pub,
    Ref,
    Return,
    Static,
    Struct,
    Super,
    Trait,
    Type,
    Unsafe,
    Use,
    Where,
    While,

    Plus,
    Minus,
    Star,
    Slash,
    Percent,
    Caret,
    Bang,
    Ampersand,
    Pipe,
    AndAnd,
    OrOr,
    Shl,
    Shr,
    PlusEq,
    MinusEq,
    StarEq,
    SlashEq,
    PercentEq,
    CaretEq,
    AmpEq,
    PipeEq,
    ShlEq,
    ShrEq,
    Eq,
    EqEq,
    BangEq,
    Lt,
    LtEq,
    Gt,
    GtEq,
    At,
    Dot,
    DotDot,
    DotDotEq,
    Comma,
    Semicolon,
    Colon,
    ColonColon,
    Arrow,
    FatArrow,
    Pound,
    Question,
    Tilde,

    LParen,
    RParen,
    LBrace,
    RBrace,
    LBracket,
    RBracket,

    Eof,
}

impl TokenKind {
    pub fn keyword(word: &str) -> Option<TokenKind> {
        Some(match word {
            "as" => Self::As,
            "break" => Self::Break,
            "const" => Self::Const,
            "continue" => Self::Continue,
            "crate" => Self::Crate,
            "dyn" => Self::Dyn,
            "else" => Self::Else,
            "enum" => Self::Enum,
            "extern" => Self::Extern,
            "false" => Self::False,
            "fn" => Self::Fn,
            "for" => Self::For,
            "if" => Self::If,
            "impl" => Self::Impl,
            "in" => Self::In,
            "let" => Self::Let,
            "loop" => Self::Loop,
            "match" => Self::Match,
            "mod" => Self::Mod,
            "move" => Self::Move,
            "mut" => Self::Mut,
            "nogc" => Self::Nogc,
            "pub" => Self::Pub,
            "ref" => Self::Ref,
            "return" => Self::Return,
            "static" => Self::Static,
            "struct" => Self::Struct,
            "super" => Self::Super,
            "trait" => Self::Trait,
            "true" => Self::True,
            "type" => Self::Type,
            "unsafe" => Self::Unsafe,
            "use" => Self::Use,
            "where" => Self::Where,
            "while" => Self::While,
            _ => return None,
        })
    }

    pub fn keyword_spelling(&self) -> Option<&'static str> {
        Some(match self {
            Self::As => "as",
            Self::Break => "break",
            Self::Const => "const",
            Self::Continue => "continue",
            Self::Crate => "crate",
            Self::Dyn => "dyn",
            Self::Else => "else",
            Self::Enum => "enum",
            Self::Extern => "extern",
            Self::False => "false",
            Self::Fn => "fn",
            Self::For => "for",
            Self::If => "if",
            Self::Impl => "impl",
            Self::In => "in",
            Self::Let => "let",
            Self::Loop => "loop",
            Self::Match => "match",
            Self::Mod => "mod",
            Self::Move => "move",
            Self::Mut => "mut",
            Self::Nogc => "nogc",
            Self::Pub => "pub",
            Self::Ref => "ref",
            Self::Return => "return",
            Self::Static => "static",
            Self::Struct => "struct",
            Self::Super => "super",
            Self::Trait => "trait",
            Self::True => "true",
            Self::Type => "type",
            Self::Unsafe => "unsafe",
            Self::Use => "use",
            Self::Where => "where",
            Self::While => "while",
            _ => return None,
        })
    }

    pub fn is_literal(&self) -> bool {
        matches!(
            self,
            Self::Int(_)
                | Self::UInt(_)
                | Self::Float(_)
                | Self::SuffixedInt(..)
                | Self::SuffixedFloat(..)
                | Self::String(_)
                | Self::Char(_)
                | Self::Byte(_)
                | Self::True
                | Self::False
        )
    }

    pub fn is_reserved_word(&self) -> bool {
        self.keyword_spelling().is_some()
    }

    fn punct(&self) -> Option<&'static str> {
        Some(match self {
            Self::Plus => "+",
            Self::Minus => "-",
            Self::Star => "*",
            Self::Slash => "/",
            Self::Percent => "%",
            Self::Caret => "^",
            Self::Bang => "!",
            Self::Ampersand => "&",
            Self::Pipe => "|",
            Self::AndAnd => "&&",
            Self::OrOr => "||",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::PlusEq => "+=",
            Self::MinusEq => "-=",
            Self::StarEq => "*=",
            Self::SlashEq => "/=",
            Self::PercentEq => "%=",
            Self::CaretEq => "^=",
            Self::AmpEq => "&=",
            Self::PipeEq => "|=",
            Self::ShlEq => "<<=",
            Self::ShrEq => ">>=",
            Self::Eq => "=",
            Self::EqEq => "==",
            Self::BangEq => "!=",
            Self::Lt => "<",
            Self::LtEq => "<=",
            Self::Gt => ">",
            Self::GtEq => ">=",
            Self::At => "@",
            Self::Dot => ".",
            Self::DotDot => "..",
            Self::DotDotEq => "..=",
            Self::Comma => ",",
            Self::Semicolon => ";",
            Self::Colon => ":",
            Self::ColonColon => "::",
            Self::Arrow => "->",
            Self::FatArrow => "=>",
            Self::Pound => "#",
            Self::Question => "?",
            Self::Tilde => "~",
            Self::LParen => "(",
            Self::RParen => ")",
            Self::LBrace => "{",
            Self::RBrace => "}",
            Self::LBracket => "[",
            Self::RBracket => "]",
            _ => return None,
        })
    }
}

impl std::fmt::Display for TokenKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if let Some(word) = self.keyword_spelling() {
            return write!(f, "`{word}`");
        }
        if let Some(p) = self.punct() {
            return write!(f, "`{p}`");
        }
        match self {
            Self::Int(n) => write!(f, "`{n}`"),
            Self::UInt(n) => write!(f, "`{n}`"),
            Self::Float(n) => write!(f, "`{n}`"),
            Self::SuffixedInt(n, s) => write!(f, "`{n}{s}`"),
            Self::SuffixedFloat(n, s) => write!(f, "`{n}{s}`"),
            Self::String(s) => write!(f, "\"{s}\""),
            Self::Char(cp) => match char::from_u32(*cp) {
                Some(c) => write!(f, "'{c}'"),
                None => write!(f, "'\\u{{{cp:x}}}'"),
            },
            Self::Byte(b) => write!(f, "b'{}'", *b as char),
            Self::Lifetime(name) => write!(f, "`'{name}`"),
            Self::Identifier(s) => write!(f, "`{s}`"),
            Self::Eof => write!(f, "end of file"),
            _ => unreachable!("keywords and punctuation are spelled above"),
        }
    }
}

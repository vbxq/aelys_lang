use super::{Lexer, Result};
use aelys_common::error::{AelysError, CompileErrorKind};
use aelys_syntax::TokenKind;

impl Lexer {
    pub(super) fn scan_token(&mut self) -> Result<()> {
        let c = self.advance();
        let kind = match c {
            ' ' | '\r' | '\t' | '\n' => return Ok(()),
            '(' => TokenKind::LParen,
            ')' => TokenKind::RParen,
            '{' => TokenKind::LBrace,
            '}' => TokenKind::RBrace,
            '[' => TokenKind::LBracket,
            ']' => TokenKind::RBracket,
            ',' => TokenKind::Comma,
            ';' => TokenKind::Semicolon,
            '@' => TokenKind::At,
            '?' => TokenKind::Question,
            '#' => TokenKind::Pound,
            '~' => TokenKind::Tilde,
            '.' => {
                if self.match_char('.') {
                    if self.match_char('=') {
                        TokenKind::DotDotEq
                    } else {
                        TokenKind::DotDot
                    }
                } else {
                    TokenKind::Dot
                }
            }
            '+' => self.with_eq(TokenKind::PlusEq, TokenKind::Plus),
            '*' => self.with_eq(TokenKind::StarEq, TokenKind::Star),
            '%' => self.with_eq(TokenKind::PercentEq, TokenKind::Percent),
            '^' => self.with_eq(TokenKind::CaretEq, TokenKind::Caret),
            '-' => {
                if self.match_char('>') {
                    TokenKind::Arrow
                } else {
                    self.with_eq(TokenKind::MinusEq, TokenKind::Minus)
                }
            }
            ':' => {
                if self.match_char(':') {
                    TokenKind::ColonColon
                } else {
                    TokenKind::Colon
                }
            }
            '/' => {
                if self.match_char('/') {
                    while self.peek() != '\n' && !self.is_at_end() {
                        self.advance();
                    }
                    return Ok(());
                }
                if self.match_char('*') {
                    return self.block_comment();
                }
                self.with_eq(TokenKind::SlashEq, TokenKind::Slash)
            }
            '=' => {
                if self.match_char('=') {
                    TokenKind::EqEq
                } else if self.match_char('>') {
                    TokenKind::FatArrow
                } else {
                    TokenKind::Eq
                }
            }
            '!' => self.with_eq(TokenKind::BangEq, TokenKind::Bang),
            '<' => {
                if self.match_char('<') {
                    self.with_eq(TokenKind::ShlEq, TokenKind::Shl)
                } else {
                    self.with_eq(TokenKind::LtEq, TokenKind::Lt)
                }
            }
            '>' => {
                if self.match_char('>') {
                    self.with_eq(TokenKind::ShrEq, TokenKind::Shr)
                } else {
                    self.with_eq(TokenKind::GtEq, TokenKind::Gt)
                }
            }
            '&' => {
                if self.match_char('&') {
                    TokenKind::AndAnd
                } else {
                    self.with_eq(TokenKind::AmpEq, TokenKind::Ampersand)
                }
            }
            '|' => {
                if self.match_char('|') {
                    TokenKind::OrOr
                } else {
                    self.with_eq(TokenKind::PipeEq, TokenKind::Pipe)
                }
            }
            '"' => return self.string(),
            '\'' => return self.quote(),
            c if c.is_ascii_digit() => return self.number(),
            'r' if self.peek() == '"' || (self.peek() == '#' && matches!(self.peek_next(), '"' | '#')) => {
                return self.raw_string();
            }
            'r' if self.peek() == '#' && is_ident_start(self.peek_next()) => {
                self.raw_identifier();
                return Ok(());
            }
            'b' if self.peek() == '\'' => {
                self.advance();
                return self.byte();
            }
            'b' if self.peek() == '"' || (self.peek() == 'r' && matches!(self.peek_next(), '"' | '#')) => {
                return Err(AelysError::Compile(self.rule(
                    128,
                    "byte strings are not supported; write a string literal and call `.as_bytes()`"
                        .to_string(),
                    self.span(),
                )));
            }
            c if is_ident_start(c) => {
                self.identifier();
                return Ok(());
            }
            _ => {
                return Err(AelysError::Compile(
                    self.error(CompileErrorKind::InvalidCharacter(c)),
                ));
            }
        };
        self.add_token(kind);
        Ok(())
    }

    fn with_eq(&mut self, with: TokenKind, without: TokenKind) -> TokenKind {
        if self.match_char('=') { with } else { without }
    }
}

pub(super) fn is_ident_start(c: char) -> bool {
    c.is_alphabetic() || c == '_'
}

pub(super) fn is_ident_continue(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

use super::{Lexer, Result};
use aelys_common::error::{AelysError, CompileErrorKind};
use aelys_syntax::TokenKind;

impl Lexer {
    pub(super) fn scan_token(&mut self) -> Result<()> {
        let c = self.advance();

        match c {
            ' ' | '\r' | '\t' => {}

            '\n' => {
                if self.pending_semicolon {
                    self.add_token(TokenKind::Semicolon);
                    self.pending_semicolon = false;
                }
                self.line += 1;
                self.column = 1;
            }

            '(' => self.add_token(TokenKind::LParen),
            ')' => self.add_token(TokenKind::RParen),
            '{' => self.add_token(TokenKind::LBrace),
            '}' => self.add_token(TokenKind::RBrace),
            '[' => self.add_token(TokenKind::LBracket),
            ']' => self.add_token(TokenKind::RBracket),
            ',' => self.add_token(TokenKind::Comma),
            ';' => self.add_token(TokenKind::Semicolon),
            '.' => {
                if self.match_char('.') {
                    if self.match_char('=') {
                        self.add_token(TokenKind::DotDotEq);
                    } else {
                        self.add_token(TokenKind::DotDot);
                    }
                } else {
                    self.add_token(TokenKind::Dot);
                }
            }
            '@' => self.add_token(TokenKind::At),

            '+' => self.add_token(TokenKind::Plus),
            '-' => {
                if self.match_char('>') {
                    self.add_token(TokenKind::Arrow);
                } else {
                    self.add_token(TokenKind::Minus);
                }
            }
            '*' => self.add_token(TokenKind::Star),
            '%' => self.add_token(TokenKind::Percent),
            ':' => self.add_token(TokenKind::Colon),

            '/' => {
                if self.match_char('/') {
                    while self.peek() != '\n' && !self.is_at_end() {
                        self.advance();
                    }
                } else if self.match_char('*') {
                    self.block_comment()?;
                } else {
                    self.add_token(TokenKind::Slash);
                }
            }

            '=' => {
                if self.match_char('=') {
                    self.add_token(TokenKind::EqEq);
                } else {
                    self.add_token(TokenKind::Eq);
                }
            }

            '!' => {
                if self.match_char('=') {
                    self.add_token(TokenKind::BangEq);
                } else {
                    return Err(AelysError::Compile(
                        self.error(CompileErrorKind::InvalidCharacter(c)),
                    ));
                }
            }

            '<' => {
                if self.match_char('<') {
                    self.add_token(TokenKind::Shl);
                } else if self.match_char('=') {
                    self.add_token(TokenKind::LtEq);
                } else {
                    self.add_token(TokenKind::Lt);
                }
            }

            '>' => {
                if self.match_char('>') {
                    self.add_token(TokenKind::Shr);
                } else if self.match_char('=') {
                    self.add_token(TokenKind::GtEq);
                } else {
                    self.add_token(TokenKind::Gt);
                }
            }

            '&' => {
                if self.match_char('&') {
                    self.add_token(TokenKind::And);
                } else {
                    self.add_token(TokenKind::Ampersand);
                }
            }
            '|' => {
                if self.match_char('|') {
                    self.add_token(TokenKind::Or);
                } else {
                    self.add_token(TokenKind::Pipe);
                }
            }
            '^' => self.add_token(TokenKind::Caret),
            '~' => self.add_token(TokenKind::Tilde),

            '"' => self.string()?,

            c if c.is_ascii_digit() => self.number()?,

            c if c.is_alphabetic() || c == '_' => self.identifier(),

            _ => {
                return Err(AelysError::Compile(
                    self.error(CompileErrorKind::InvalidCharacter(c)),
                ));
            }
        }

        Ok(())
    }
}

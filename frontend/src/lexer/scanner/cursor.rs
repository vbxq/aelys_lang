use super::Lexer;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::{Span, Token, TokenKind};
use std::sync::Arc;

impl Lexer {
    pub(super) fn add_token(&mut self, kind: TokenKind) {
        self.pending_semicolon = kind.can_end_statement();
        let span = Span::new(self.start, self.current, self.line, self.start_column);
        self.tokens.push(Token::new(kind, span));
    }

    pub(super) fn advance(&mut self) -> char {
        let c = self.chars.get(self.current).copied().unwrap_or('\0');
        self.current += 1;
        self.column += 1;
        c
    }

    pub(super) fn peek(&self) -> char {
        self.chars.get(self.current).copied().unwrap_or('\0')
    }

    pub(super) fn peek_next(&self) -> char {
        self.chars.get(self.current + 1).copied().unwrap_or('\0')
    }

    pub(super) fn match_char(&mut self, expected: char) -> bool {
        if self.is_at_end() || self.chars[self.current] != expected {
            return false;
        }
        self.current += 1;
        self.column += 1;
        true
    }

    pub(super) fn is_at_end(&self) -> bool {
        self.current >= self.chars.len()
    }

    pub(super) fn next_token_is_else(&self) -> bool {
        let mut i = self.current;
        // skip whitespace, newlines, and comments to find the next real token
        while i < self.chars.len() {
            let c = self.chars[i];
            if c == ' ' || c == '\t' || c == '\r' || c == '\n' {
                i += 1;
            } else if i + 1 < self.chars.len() && c == '/' && self.chars[i + 1] == '/' {
                // skip line comment
                i += 2;
                while i < self.chars.len() && self.chars[i] != '\n' {
                    i += 1;
                }
            } else if i + 1 < self.chars.len() && c == '/' && self.chars[i + 1] == '*' {
                // skip block comment (with nesting)
                i += 2;
                let mut depth = 1u32;
                while i + 1 < self.chars.len() && depth > 0 {
                    if self.chars[i] == '/' && self.chars[i + 1] == '*' {
                        depth += 1;
                        i += 2;
                    } else if self.chars[i] == '*' && self.chars[i + 1] == '/' {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
            } else {
                break;
            }
        }
        // check for "else"
        if i + 4 <= self.chars.len() {
            let word: String = self.chars[i..i + 4].iter().collect();
            if word == "else" {
                // make sure it's not a prefix of another identifier
                let next = self.chars.get(i + 4).copied().unwrap_or('\0');
                return !next.is_alphanumeric() && next != '_';
            }
        }
        false
    }

    pub(super) fn error(&self, kind: CompileErrorKind) -> CompileError {
        CompileError::new(
            kind,
            Span::new(self.start, self.current, self.line, self.start_column),
            Arc::clone(&self.source),
        )
    }
}

use super::Lexer;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::{Span, Token, TokenKind};
use std::sync::Arc;

impl Lexer {
    pub(super) fn add_token(&mut self, kind: TokenKind) {
        let span = self.span();
        self.tokens.push(Token::new(kind, span));
    }

    pub(super) fn span(&self) -> Span {
        Span::new(self.start, self.current, self.start_line, self.start_column)
    }

    pub(super) fn text(&self, from: usize) -> &str {
        &self.source.content[from..self.current]
    }

    pub(super) fn advance(&mut self) -> char {
        let c = self.peek();
        if self.is_at_end() {
            return c;
        }
        self.current += c.len_utf8();
        if c == '\n' {
            self.line += 1;
            self.column = 1;
        } else {
            self.column += 1;
        }
        c
    }

    pub(super) fn peek(&self) -> char {
        self.source.content[self.current..]
            .chars()
            .next()
            .unwrap_or('\0')
    }

    pub(super) fn peek_next(&self) -> char {
        self.source.content[self.current..]
            .chars()
            .nth(1)
            .unwrap_or('\0')
    }

    pub(super) fn peek_at(&self, n: usize) -> char {
        self.source.content[self.current..]
            .chars()
            .nth(n)
            .unwrap_or('\0')
    }

    pub(super) fn match_char(&mut self, expected: char) -> bool {
        if self.is_at_end() || self.peek() != expected {
            return false;
        }
        self.advance();
        true
    }

    pub(super) fn is_at_end(&self) -> bool {
        self.current >= self.source.content.len()
    }

    pub(super) fn error(&self, kind: CompileErrorKind) -> CompileError {
        CompileError::new(kind, self.span(), Arc::clone(&self.source))
    }

    pub(super) fn rule(&self, code: u16, message: String, span: Span) -> CompileError {
        CompileError::new(
            CompileErrorKind::ParseRule {
                code,
                message,
                help: None,
            },
            span,
            Arc::clone(&self.source),
        )
    }
}

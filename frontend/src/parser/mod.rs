mod decl;
mod expr;
mod stmt;

use aelys_common::Diagnostic;
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_syntax::Source;
use aelys_syntax::{Stmt, Token, TokenKind};
use std::sync::Arc;

const MAX_RECURSION_DEPTH: usize = 1000; // pathological nesting guard

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
    pub(crate) source: Arc<Source>,
    recursion_depth: usize,
    pub(crate) block_depth: usize,
    errors: Vec<Diagnostic>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>, source: Arc<Source>) -> Self {
        Self {
            tokens,
            current: 0,
            source,
            recursion_depth: 0,
            block_depth: 0,
            errors: Vec::new(),
        }
    }

    pub fn parse(mut self) -> Result<Vec<Stmt>> {
        let mut statements = Vec::new();

        let mut prologue_open = true;

        while !self.is_at_end() {
            if self.match_token(&TokenKind::Semicolon) {
                continue;
            }

            let parsed = if prologue_open && self.check(&TokenKind::Needs) {
                self.needs_declaration()
            } else {
                prologue_open = false;
                self.declaration()
            };

            match parsed {
                Ok(stmt) => statements.push(stmt),
                Err(err) => {
                    let diag = match &err {
                        AelysError::Compile(e) => e.to_diagnostic(),
                        AelysError::Multiple(diags) => {
                            if let Some(d) = diags.first() {
                                d.clone()
                            } else {
                                continue;
                            }
                        }
                    };
                    self.errors.push(diag);
                    self.synchronize();
                }
            }
        }

        if self.errors.is_empty() {
            Ok(statements)
        } else {
            Err(AelysError::Multiple(self.errors))
        }
    }

    fn synchronize(&mut self) {
        while !self.is_at_end() {
            if self.peek().kind == TokenKind::Semicolon {
                self.advance();
                return;
            }

            match &self.peek().kind {
                TokenKind::Let
                | TokenKind::Fn
                | TokenKind::If
                | TokenKind::While
                | TokenKind::For
                | TokenKind::Return
                | TokenKind::Struct
                | TokenKind::Enum
                | TokenKind::Match
                | TokenKind::Pub => return,
                TokenKind::Eof => return,
                _ => {}
            }

            self.advance();
        }
    }

    pub fn error(&self, kind: CompileErrorKind) -> aelys_common::error::AelysError {
        CompileError::new(kind, self.peek().span, Arc::clone(&self.source)).into()
    }

    pub(crate) fn enter_recursion(&mut self) -> Result<()> {
        self.recursion_depth += 1;
        if self.recursion_depth > MAX_RECURSION_DEPTH {
            return Err(self.error(CompileErrorKind::RecursionDepthExceeded {
                max: MAX_RECURSION_DEPTH,
            }));
        }
        Ok(())
    }

    pub(crate) fn exit_recursion(&mut self) {
        self.recursion_depth = self.recursion_depth.saturating_sub(1);
    }

    fn consume(&mut self, kind: &TokenKind, expected: &str) -> Result<()> {
        if self.check(kind) {
            self.advance();
            Ok(())
        } else {
            Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: expected.to_string(),
                found: self.peek().kind.to_string(),
            }))
        }
    }

    fn consume_gt(&mut self) -> Result<()> {
        if self.check(&TokenKind::Gt) {
            self.advance();
            Ok(())
        } else if self.check(&TokenKind::Shr) {
            let span = self.tokens[self.current].span;
            let first_gt_span = aelys_syntax::Span {
                start: span.start,
                end: span.start + 1,
                line: span.line,
                column: span.column,
            };
            let second_gt_span = aelys_syntax::Span {
                start: span.start + 1,
                end: span.end,
                line: span.line,
                column: span.column + 1,
            };
            self.tokens[self.current] = Token::new(TokenKind::Gt, second_gt_span);
            self.tokens
                .insert(self.current, Token::new(TokenKind::Gt, first_gt_span));
            self.advance();
            Ok(())
        } else {
            Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: ">".to_string(),
                found: self.peek().kind.to_string(),
            }))
        }
    }

    fn consume_identifier(&mut self, expected: &str) -> Result<String> {
        match &self.peek().kind {
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: expected.to_string(),
                found: self.peek().kind.to_string(),
            })),
        }
    }

    // while null stays a reserved keyword everywhere else
    fn consume_path_segment(&mut self, expected: &str) -> Result<String> {
        if matches!(self.peek().kind, TokenKind::Null) {
            self.advance();
            return Ok("null".to_string());
        }
        self.consume_identifier(expected)
    }

    fn check(&self, kind: &TokenKind) -> bool {
        if self.is_at_end() {
            return false;
        }
        self.peek().kind == *kind
    }

    fn match_token(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn peek_at(&self, offset: usize) -> &Token {
        let idx = self.current + offset;
        if idx < self.tokens.len() {
            &self.tokens[idx]
        } else {
            self.tokens.last().unwrap()
        }
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.current - 1]
    }
}

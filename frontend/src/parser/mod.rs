// recursive descent parser

mod decl;
mod expr;
mod stmt;

use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::Source;
use aelys_syntax::{Stmt, Token, TokenKind};
use std::sync::Arc;

const MAX_RECURSION_DEPTH: usize = 1000; // pathological nesting guard

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
    pub(crate) source: Arc<Source>,
    recursion_depth: usize,
}

impl Parser {
    pub fn new(tokens: Vec<Token>, source: Arc<Source>) -> Self {
        Self {
            tokens,
            current: 0,
            source,
            recursion_depth: 0,
        }
    }

    pub fn parse(mut self) -> Result<Vec<Stmt>> {
        let mut statements = Vec::new();

        while !self.is_at_end() {
            if self.match_token(&TokenKind::Semicolon) {
                continue;
            }

            statements.push(self.declaration()?);
        }

        Ok(statements)
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

// lexer: chars -> tokens
// TODO: consider switching to logos for speed

use aelys_common::error::AelysError;
use aelys_syntax::{Source, Token, TokenKind};
use std::sync::Arc;

mod comment;
mod cursor;
mod identifier;
mod number;
mod scan;
mod string;

type Result<T> = std::result::Result<T, AelysError>;

const MAX_COMMENT_DEPTH: usize = 256; // nested /* */ limit

pub struct Lexer {
    source: Arc<Source>,
    chars: Vec<char>,
    tokens: Vec<Token>,
    start: usize,
    current: usize,
    line: u32,
    column: u32,
    start_column: u32,
    pending_semicolon: bool,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self {
            source: Source::new("<input>", source),
            chars: source.chars().collect(),
            tokens: Vec::new(),
            start: 0,
            current: 0,
            line: 1,
            column: 1,
            start_column: 1,
            pending_semicolon: false,
        }
    }

    pub fn with_source(source: Arc<Source>) -> Self {
        let chars = source.content.chars().collect();
        Self {
            source,
            chars,
            tokens: Vec::new(),
            start: 0,
            current: 0,
            line: 1,
            column: 1,
            start_column: 1,
            pending_semicolon: false,
        }
    }

    pub fn scan(mut self) -> Result<Vec<Token>> {
        while !self.is_at_end() {
            self.start = self.current;
            self.start_column = self.column;
            self.scan_token()?;
        }

        if self.pending_semicolon {
            self.add_token(TokenKind::Semicolon);
        }

        self.add_token(TokenKind::Eof);
        Ok(self.tokens)
    }
}

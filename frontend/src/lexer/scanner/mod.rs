use aelys_common::error::AelysError;
use aelys_syntax::{Source, Token, TokenKind};
use std::sync::Arc;

mod character;
mod comment;
mod cursor;
mod identifier;
mod number;
mod scan;
mod string;

type Result<T> = std::result::Result<T, AelysError>;

const MAX_COMMENT_DEPTH: usize = 256;

pub struct Lexer {
    source: Arc<Source>,
    tokens: Vec<Token>,
    start: usize,
    current: usize,
    line: u32,
    column: u32,
    start_line: u32,
    start_column: u32,
}

impl Lexer {
    pub fn new(source: &str) -> Self {
        Self::with_source(Source::new("<input>", source))
    }

    pub fn with_source(source: Arc<Source>) -> Self {
        Self {
            source,
            tokens: Vec::new(),
            start: 0,
            current: 0,
            line: 1,
            column: 1,
            start_line: 1,
            start_column: 1,
        }
    }

    pub fn scan(mut self) -> Result<Vec<Token>> {
        loop {
            self.start = self.current;
            self.start_line = self.line;
            self.start_column = self.column;
            if self.is_at_end() {
                break;
            }
            self.scan_token()?;
        }
        self.add_token(TokenKind::Eof);
        Ok(self.tokens)
    }
}

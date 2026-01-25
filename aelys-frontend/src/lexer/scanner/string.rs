use super::{Lexer, Result};
use aelys_common::error::{AelysError, CompileErrorKind};
use aelys_syntax::TokenKind;

impl Lexer {
    pub(super) fn string(&mut self) -> Result<()> {
        let mut value = String::new();

        while self.peek() != '"' && !self.is_at_end() {
            if self.peek() == '\n' {
                self.line += 1;
                self.column = 1;
            }
            if self.peek() == '\\' {
                self.advance();
                match self.peek() {
                    'n' => {
                        self.advance();
                        value.push('\n');
                    }
                    't' => {
                        self.advance();
                        value.push('\t');
                    }
                    'r' => {
                        self.advance();
                        value.push('\r');
                    }
                    '\\' => {
                        self.advance();
                        value.push('\\');
                    }
                    '"' => {
                        self.advance();
                        value.push('"');
                    }
                    '0' => {
                        self.advance();
                        value.push('\0');
                    }
                    '\'' => {
                        self.advance();
                        value.push('\'');
                    }
                    c => {
                        return Err(AelysError::Compile(
                            self.error(CompileErrorKind::InvalidEscape(c)),
                        ));
                    }
                }
            } else {
                value.push(self.advance());
            }
        }

        if self.is_at_end() {
            return Err(AelysError::Compile(
                self.error(CompileErrorKind::UnterminatedString),
            ));
        }

        self.advance();
        self.add_token(TokenKind::String(value));
        Ok(())
    }
}

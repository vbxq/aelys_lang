use super::{Lexer, Result};
use aelys_common::error::{AelysError, CompileErrorKind};
use aelys_syntax::TokenKind;

impl Lexer {
    pub(super) fn character(&mut self) -> Result<()> {
        let mut scalars: Vec<char> = Vec::new();

        loop {
            if self.is_at_end() || self.peek() == '\n' {
                return Err(AelysError::Compile(
                    self.error(CompileErrorKind::UnterminatedCharLiteral),
                ));
            }
            if self.peek() == '\'' {
                self.advance();
                break;
            }
            if self.peek() == '\\' {
                self.advance();
                let escaped = match self.peek() {
                    'n' => '\n',
                    't' => '\t',
                    'r' => '\r',
                    '\\' => '\\',
                    '"' => '"',
                    '0' => '\0',
                    '\'' => '\'',
                    c => {
                        return Err(AelysError::Compile(
                            self.error(CompileErrorKind::InvalidEscape(c)),
                        ));
                    }
                };
                self.advance();
                scalars.push(escaped);
                continue;
            }
            scalars.push(self.advance());
        }

        if scalars.len() != 1 {
            return Err(AelysError::Compile(self.error(
                CompileErrorKind::CharLiteralNotOneScalar {
                    scalars: scalars.len(),
                },
            )));
        }

        self.add_token(TokenKind::Char(scalars[0] as u32));
        Ok(())
    }
}

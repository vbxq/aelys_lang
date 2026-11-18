use super::scan::{is_ident_continue, is_ident_start};
use super::{Lexer, Result};
use aelys_common::error::{AelysError, CompileErrorKind};
use aelys_syntax::TokenKind;

impl Lexer {
    pub(super) fn quote(&mut self) -> Result<()> {
        if self.peek() != '\\' && self.peek_next() != '\'' && is_ident_start(self.peek()) {
            let from = self.current;
            while is_ident_continue(self.peek()) {
                self.advance();
            }
            let name = self.text(from).to_string();
            self.add_token(TokenKind::Lifetime(name));
            return Ok(());
        }
        let c = self.char_body(true)?;
        self.add_token(TokenKind::Char(c as u32));
        Ok(())
    }

    pub(super) fn byte(&mut self) -> Result<()> {
        let c = self.char_body(false)?;
        if !c.is_ascii() && (c as u32) > 0xff {
            return Err(AelysError::Compile(self.rule(
                128,
                format!("`{c}` is not a byte; a byte literal holds one ASCII character"),
                self.span(),
            )));
        }
        self.add_token(TokenKind::Byte(c as u32 as u8));
        Ok(())
    }

    fn char_body(&mut self, unicode: bool) -> Result<char> {
        let mut scalars = Vec::new();
        loop {
            if self.is_at_end() || self.peek() == '\n' {
                return Err(AelysError::Compile(
                    self.error(CompileErrorKind::UnterminatedCharLiteral),
                ));
            }
            match self.peek() {
                '\'' => {
                    self.advance();
                    break;
                }
                '\\' => scalars.push(self.escape(unicode)?),
                _ => scalars.push(self.advance()),
            }
        }
        if scalars.len() != 1 {
            return Err(AelysError::Compile(self.error(
                CompileErrorKind::CharLiteralNotOneScalar {
                    scalars: scalars.len(),
                },
            )));
        }
        Ok(scalars[0])
    }
}

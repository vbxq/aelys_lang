use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Decorator, TokenKind};

impl Parser {
    pub(super) fn decorators(&mut self) -> Result<Vec<Decorator>> {
        let mut decorators = Vec::new();

        while self.match_token(&TokenKind::At) {
            let span = self.previous().span;
            let name = self.consume_identifier("decorator name")?;
            decorators.push(Decorator { name, span });
            self.consume_semicolon()?;
        }

        Ok(decorators)
    }
}

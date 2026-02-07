use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{Stmt, TokenKind};

mod decorators;
mod function;
mod let_decl;
mod needs;
mod types;

impl Parser {
    pub fn declaration(&mut self) -> Result<Stmt> {
        if self.check(&TokenKind::Needs) {
            return self.needs_declaration();
        }

        let decorators = self.decorators()?;
        let is_pub = self.match_token(&TokenKind::Pub);

        if self.check(&TokenKind::Fn) {
            return self.function_declaration(decorators, is_pub);
        }

        if !decorators.is_empty() {
            return Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: "function after decorator".to_string(),
                found: self.peek().kind.to_string(),
            }));
        }

        if self.check(&TokenKind::Let) {
            return self.let_declaration(is_pub);
        }

        if is_pub {
            return Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: "fn or let after pub".to_string(),
                found: self.peek().kind.to_string(),
            }));
        }

        self.statement()
    }
}

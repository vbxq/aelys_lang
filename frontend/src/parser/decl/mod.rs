use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{Stmt, TokenKind};

mod decorators;
mod enum_decl;
mod function;
mod let_decl;
mod needs;
mod struct_decl;
mod types;

impl Parser {
    pub fn declaration(&mut self) -> Result<Stmt> {
        if self.check(&TokenKind::Needs) {
            return self.needs_declaration();
        }

        let decorators = self.decorators()?;
        let is_pub = self.match_token(&TokenKind::Pub);

        if self.check(&TokenKind::Struct) {
            if !decorators.is_empty() {
                return Err(self.error(CompileErrorKind::UnexpectedToken {
                    expected: "function after decorator".to_string(),
                    found: self.peek().kind.to_string(),
                }));
            }
            return self.struct_declaration(is_pub);
        }

        if self.check(&TokenKind::Enum) {
            if !decorators.is_empty() {
                return Err(self.error(CompileErrorKind::UnexpectedToken {
                    expected: "function after decorator".to_string(),
                    found: self.peek().kind.to_string(),
                }));
            }
            return self.enum_declaration(is_pub);
        }

        let is_nogc = self.match_token(&TokenKind::Nogc);

        if self.check(&TokenKind::Fn) {
            return self.function_declaration(decorators, is_pub, is_nogc);
        }

        if !decorators.is_empty() {
            return Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: "function after decorator".to_string(),
                found: self.peek().kind.to_string(),
            }));
        }

        if is_nogc {
            return Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: "fn after `nogc`".to_string(),
                found: self.peek().kind.to_string(),
            }));
        }

        if self.check(&TokenKind::Let) {
            return self.let_declaration(is_pub);
        }

        if is_pub {
            return Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: "fn, let, struct, or enum after pub".to_string(),
                found: self.peek().kind.to_string(),
            }));
        }

        self.statement()
    }
}

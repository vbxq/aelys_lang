use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{Stmt, TokenKind};

mod const_decl;
mod decorators;
mod enum_decl;
mod function;
mod let_decl;
mod needs;
mod struct_decl;
mod types;

impl Parser {
    pub(crate) fn starts_item(&self) -> bool {
        let contextual = |word: &str| {
            matches!(&self.peek().kind, TokenKind::Identifier(w) if w == word)
                && matches!(
                    self.peek_at(1).kind,
                    TokenKind::Identifier(_) | TokenKind::Lt
                )
        };
        match &self.peek().kind {
            TokenKind::Fn
            | TokenKind::Struct
            | TokenKind::Enum
            | TokenKind::Pub
            | TokenKind::Const
            | TokenKind::Nogc
            | TokenKind::Extern
            | TokenKind::Needs
            | TokenKind::At
            | TokenKind::Let => true,
            TokenKind::Unsafe => self.peek_at(1).kind == TokenKind::Extern,
            _ => self.starts_type_alias() || contextual("impl") || contextual("trait"),
        }
    }

    pub fn declaration(&mut self) -> Result<Stmt> {
        if self.check(&TokenKind::Needs) {
            return Err(self.error(CompileErrorKind::NeedsOutsidePrologue));
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

        if self.check(&TokenKind::Unsafe) && self.peek_at(1).kind == TokenKind::Extern {
            return self.foreign_declaration(decorators, is_pub);
        }

        if self.check(&TokenKind::Extern) {
            return Err(self.error(CompileErrorKind::MalformedForeignDecl {
                reason: "an external declaration must be `unsafe`".to_string(),
            }));
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

        if self.check(&TokenKind::Const) {
            return self.const_declaration(is_pub);
        }

        if self.starts_type_alias() {
            return self.type_alias_declaration(is_pub);
        }

        if is_pub {
            return Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: "fn, let, const, type, struct, or enum after pub".to_string(),
                found: self.peek().kind.to_string(),
            }));
        }

        self.statement()
    }
}

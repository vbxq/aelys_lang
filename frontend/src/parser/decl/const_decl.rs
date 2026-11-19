use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Stmt, StmtKind, TokenKind};

impl Parser {
    pub(super) fn const_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance();
        if self.block_depth > 0 {
            return Err(self.rule(101, "a const is declared at module level", start_span));
        }
        let name_span = self.peek().span;
        let name = self.consume_identifier("const name")?;
        if !name.starts_with(char::is_uppercase) {
            return Err(self.rule(
                101,
                &format!(
                    "a const name starts with an uppercase letter; write `{}`",
                    name.to_uppercase()
                ),
                name_span,
            ));
        }
        if !self.match_token(&TokenKind::Colon) {
            return Err(self.rule(
                101,
                &format!("a const needs a type: `const {name}: i64 = ...`"),
                self.peek().span,
            ));
        }
        let type_annotation = self.parse_type_annotation()?;
        self.consume(&TokenKind::Eq, "=")?;
        let initializer = self.expression()?;
        self.consume_semicolon()?;
        let end_span = self.previous().span;
        Ok(Stmt::new(
            StmtKind::Const {
                name,
                type_annotation,
                initializer,
                is_pub,
            },
            start_span.merge(end_span),
        ))
    }

    pub(super) fn starts_type_alias(&self) -> bool {
        matches!(&self.peek().kind, TokenKind::Identifier(w) if w == "type")
            && matches!(self.peek_at(1).kind, TokenKind::Identifier(_))
            && matches!(self.peek_at(2).kind, TokenKind::Eq | TokenKind::Lt)
    }

    pub(super) fn type_alias_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance();
        if self.block_depth > 0 {
            return Err(self.rule(101, "a type alias is declared at module level", start_span));
        }
        let name_span = self.peek().span;
        let name = self.consume_identifier("type alias name")?;
        if !name.starts_with(char::is_uppercase) {
            return Err(self.rule(
                101,
                &format!("a type alias name is capitalized; `{name}` starts with a lowercase letter"),
                name_span,
            ));
        }
        if matches!(name.as_str(), "Option" | "Result" | "Vec" | "Rc") {
            return Err(self.rule(
                204,
                &format!("the name `{name}` is defined twice, once by the prelude and once as a type alias"),
                name_span,
            ));
        }
        let mut type_params = Vec::new();
        if self.match_token(&TokenKind::Lt) {
            type_params.push(self.consume_identifier("type parameter")?);
            while self.match_token(&TokenKind::Comma) {
                type_params.push(self.consume_identifier("type parameter")?);
            }
            self.consume_gt()?;
        }
        self.consume(&TokenKind::Eq, "=")?;
        let target = self.parse_type_annotation()?;
        if !self.check(&TokenKind::Eof)
            && !self.check(&TokenKind::RBrace)
            && self.peek().span.line == self.previous().span.line
        {
            self.consume_semicolon()?;
        } else {
            self.match_token(&TokenKind::Semicolon);
        }
        let end_span = self.previous().span;
        Ok(Stmt::new(
            StmtKind::TypeAlias {
                name,
                type_params,
                target,
                is_pub,
            },
            start_span.merge(end_span),
        ))
    }
}

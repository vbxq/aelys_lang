use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Stmt, StmtKind, TokenKind};

impl Parser {
    pub(super) fn let_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance();

        let mutable = self.match_token(&TokenKind::Mut);
        let name = self.consume_identifier("variable name")?;

        let type_annotation = if self.match_token(&TokenKind::Colon) {
            Some(self.parse_type_annotation()?)
        } else {
            None
        };

        self.consume(&TokenKind::Eq, "=")?;
        let initializer = self.expression()?;
        self.consume_semicolon()?;

        let end_span = self.previous().span;

        Ok(Stmt::new(
            StmtKind::Let {
                name,
                mutable,
                type_annotation,
                initializer,
                is_pub,
            },
            start_span.merge(end_span),
        ))
    }
}

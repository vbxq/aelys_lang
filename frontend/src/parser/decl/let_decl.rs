use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Stmt, StmtKind, TokenKind};

impl Parser {
    pub(super) fn let_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance();

        if self.check(&TokenKind::LParen) {
            return self.let_pattern(start_span);
        }

        let mutable = self.match_token(&TokenKind::Mut);
        let name = self.consume_value_name("variable name")?;

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

    fn let_pattern(&mut self, start_span: aelys_syntax::Span) -> Result<Stmt> {
        if self.block_depth == 0 {
            return Err(self.rule(
                902,
                "a tuple pattern in a module-level let is not supported",
                start_span,
            ));
        }
        let pattern = self.parse_let_pattern()?;
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
            StmtKind::LetPattern {
                pattern,
                type_annotation,
                initializer,
            },
            start_span.merge(end_span),
        ))
    }
}

use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Pattern, Stmt, StmtKind, TokenKind};

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
        let pattern = self.let_pattern_part()?;
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

    fn let_pattern_part(&mut self) -> Result<Pattern> {
        let span = self.peek().span;
        if self.match_token(&TokenKind::LParen) {
            let mut elems = vec![self.let_pattern_part()?];
            while self.match_token(&TokenKind::Comma) {
                if self.check(&TokenKind::RParen) {
                    break;
                }
                elems.push(self.let_pattern_part()?);
            }
            self.consume(&TokenKind::RParen, ")")?;
            let end_span = self.previous().span;
            if elems.len() == 1 {
                return Err(self.rule(
                    902,
                    "a one-element tuple pattern is not supported; bind the value itself",
                    span.merge(end_span),
                ));
            }
            return Ok(Pattern::Tuple {
                elems,
                span: span.merge(end_span),
            });
        }
        if matches!(&self.peek().kind, TokenKind::Identifier(n) if n == "_") {
            self.advance();
            return Ok(Pattern::Wildcard(span));
        }
        let mutable = self.match_token(&TokenKind::Mut);
        if !matches!(self.peek().kind, TokenKind::Identifier(_)) && self.peek().kind.keyword_spelling().is_none() {
            return Err(self.rule(
                101,
                "a let pattern cannot fail; use match",
                self.peek().span,
            ));
        }
        let name = self.consume_binding_name("binding name")?;
        let end_span = self.previous().span;
        Ok(Pattern::Binding {
            name,
            mutable,
            span: span.merge(end_span),
        })
    }
}

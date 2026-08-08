use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Decorator, Function, Stmt, StmtKind, TokenKind};

impl Parser {
    pub(super) fn function_declaration(
        &mut self,
        decorators: Vec<Decorator>,
        is_pub: bool,
        is_nogc: bool,
    ) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance();

        let name = self.consume_identifier("function name")?;

        let (type_params, nogc_bounds) = if self.match_token(&TokenKind::Lt) {
            let mut params = Vec::new();
            let mut bounds = Vec::new();
            loop {
                params.push(self.consume_identifier("type parameter")?);
                bounds.push(if self.match_token(&TokenKind::Colon) {
                    self.consume(&TokenKind::Nogc, "`nogc`")?;
                    true
                } else {
                    false
                });
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume(&TokenKind::Gt, ">")?;
            (params, bounds)
        } else {
            (Vec::new(), Vec::new())
        };

        self.consume(&TokenKind::LParen, "(")?;

        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                params.push(self.parse_parameter()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RParen) {
                    break;
                }
            }
        }

        self.consume(&TokenKind::RParen, ")")?;

        let return_type = if self.match_token(&TokenKind::Arrow) {
            Some(self.parse_type_annotation()?)
        } else {
            None
        };

        self.consume(&TokenKind::LBrace, "{")?;

        let body = self.block_statements()?;
        let end_span = self.previous().span;

        let function = Function {
            name: name.clone(),
            type_params,
            nogc_bounds,
            params,
            return_type,
            body,
            decorators,
            is_pub,
            is_nogc,
            span: start_span.merge(end_span),
        };

        Ok(Stmt::new(
            StmtKind::Function(function),
            start_span.merge(end_span),
        ))
    }
}

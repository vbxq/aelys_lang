use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{Decorator, ForeignConv, ForeignDecl, Function, Stmt, StmtKind, TokenKind};

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
            foreign: None,
            span: start_span.merge(end_span),
        };

        Ok(Stmt::new(
            StmtKind::Function(function),
            start_span.merge(end_span),
        ))
    }

    pub(super) fn foreign_declaration(
        &mut self,
        decorators: Vec<Decorator>,
        is_pub: bool,
    ) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance();
        self.advance();

        if is_pub {
            return Err(self.malformed_foreign("an external declaration cannot be `pub`"));
        }
        if !decorators.is_empty() {
            return Err(self.malformed_foreign("an external declaration carries no decorator"));
        }
        if self.block_depth > 0 {
            return Err(self.malformed_foreign("an external declaration belongs at the top level"));
        }

        let is_nogc = self.match_token(&TokenKind::Nogc);
        self.consume(&TokenKind::Fn, "`nogc` or `fn` after `extern`")?;

        let name = self.consume_identifier("function name")?;

        if self.check(&TokenKind::Lt) {
            return Err(self.malformed_foreign("an external declaration takes no type parameter"));
        }

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

        if self.check(&TokenKind::LBrace) {
            return Err(self.malformed_foreign("an external declaration has no body"));
        }

        let end_span = self.previous().span;
        let span = start_span.merge(end_span);

        let function = Function {
            name: name.clone(),
            type_params: Vec::new(),
            nogc_bounds: Vec::new(),
            params,
            return_type,
            body: Vec::new(),
            decorators: Vec::new(),
            is_pub: false,
            is_nogc,
            foreign: Some(ForeignDecl {
                symbol: name,
                calling_conv: ForeignConv::C,
                is_unsafe: true,
                span,
            }),
            span,
        };

        Ok(Stmt::new(StmtKind::Function(function), span))
    }

    fn malformed_foreign(&self, reason: &str) -> aelys_common::error::AelysError {
        self.error(CompileErrorKind::MalformedForeignDecl {
            reason: reason.to_string(),
        })
    }
}

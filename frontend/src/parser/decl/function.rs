use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{
    Decorator, ForeignConv, ForeignDecl, Function, Receiver, Stmt, StmtKind, TokenKind, TypeBounds,
};

impl Parser {
    pub(super) fn parse_type_bound(&mut self, set: &mut TypeBounds) -> Result<()> {
        if self.match_token(&TokenKind::Nogc) {
            set.nogc = true;
            return Ok(());
        }
        let TokenKind::Identifier(name) = self.peek().kind.clone() else {
            return Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: "a bound name".to_string(),
                found: self.peek().kind.to_string(),
            }));
        };
        if !set.set(&name) {
            return Err(self.error(CompileErrorKind::UnknownTypeBound { name }));
        }
        self.advance();
        Ok(())
    }

    pub(super) fn function_declaration(
        &mut self,
        decorators: Vec<Decorator>,
        is_pub: bool,
        is_nogc: bool,
    ) -> Result<Stmt> {
        let (function, _) = self.function_item(decorators, is_pub, is_nogc, false)?;
        let span = function.span;
        Ok(Stmt::new(StmtKind::Function(function), span))
    }

    pub(super) fn self_param_message() -> String {
        "a `self` parameter is only valid in a method of an `impl` block, as its first parameter"
            .to_string()
    }

    pub(super) fn receiver_ahead(&self) -> Option<(Receiver, usize)> {
        let at = |i: usize| &self.peek_at(i).kind;
        let is_self = |k: &TokenKind| matches!(k, TokenKind::Identifier(n) if n == "self");
        if is_self(at(0)) {
            return Some((Receiver::Value, 1));
        }
        if *at(0) == TokenKind::Ampersand {
            if is_self(at(1)) {
                return Some((Receiver::Ref, 2));
            }
            if *at(1) == TokenKind::Mut && is_self(at(2)) {
                return Some((Receiver::RefMut, 3));
            }
        }
        if *at(0) == TokenKind::Mut && is_self(at(1)) {
            return Some((Receiver::Value, 2));
        }
        None
    }

    pub(super) fn function_item(
        &mut self,
        decorators: Vec<Decorator>,
        is_pub: bool,
        is_nogc: bool,
        allow_receiver: bool,
    ) -> Result<(Function, Receiver)> {
        let start_span = self.peek().span;
        self.advance();

        let name = self.consume_value_name("function name")?;

        let (type_params, bounds) = if self.match_token(&TokenKind::Lt) {
            let mut params = Vec::new();
            let mut bounds = Vec::new();
            loop {
                params.push(self.consume_identifier("type parameter")?);
                let mut set = TypeBounds::default();
                if self.match_token(&TokenKind::Colon) {
                    loop {
                        self.parse_type_bound(&mut set)?;
                        if !self.match_token(&TokenKind::Plus) {
                            break;
                        }
                    }
                }
                bounds.push(set);
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

        let mut receiver = Receiver::None;
        if let Some((kind, len)) = self.receiver_ahead() {
            if !allow_receiver {
                return Err(self.error(CompileErrorKind::MethodSyntax {
                    message: Self::self_param_message(),
                }));
            }
            if self.peek_at(0).kind == TokenKind::Mut {
                return Err(self.error(CompileErrorKind::MethodSyntax {
                    message: "a `mut self` receiver is not supported; take `self` and write \
                              `let mut s = self` in the body"
                        .to_string(),
                }));
            }
            for _ in 0..len {
                self.advance();
            }
            if self.check(&TokenKind::Colon) {
                return Err(self.error(CompileErrorKind::MethodSyntax {
                    message: "a `self` receiver takes no type annotation; write `self`, `&self` \
                              or `&mut self`"
                        .to_string(),
                }));
            }
            receiver = kind;
            if !self.check(&TokenKind::RParen) {
                self.consume(&TokenKind::Comma, ", or )")?;
            }
        }

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
            bounds,
            params,
            return_type,
            body,
            decorators,
            is_pub,
            is_nogc,
            foreign: None,
            span: start_span.merge(end_span),
        };

        Ok((function, receiver))
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

        let name = self.consume_value_name("function name")?;

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
            bounds: Vec::new(),
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

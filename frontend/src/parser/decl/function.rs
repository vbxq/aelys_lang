use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{
    Decorator, ForeignConv, ForeignDecl, Function, Receiver, Stmt, StmtKind, TokenKind, TraitRef,
    TypeBounds,
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
        let span = self.peek().span;
        self.advance();
        if set.set(&name) {
            return Ok(());
        }
        let mut path = name;
        while self.check(&TokenKind::Dot)
            && matches!(self.peek_at(1).kind, TokenKind::Identifier(_))
        {
            self.advance();
            path.push('.');
            path.push_str(&self.consume_identifier("trait name segment")?);
        }
        let mut args = Vec::new();
        if self.match_token(&TokenKind::Lt) {
            loop {
                args.push(self.parse_type_annotation()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume_gt()?;
        }
        let span = span.merge(self.previous().span);
        let r = TraitRef {
            name: path,
            span,
            args,
        };
        if !set.traits.contains(&r) {
            set.traits.push(r);
        }
        Ok(())
    }

    pub(super) fn parse_bound_list(&mut self) -> Result<TypeBounds> {
        let mut set = TypeBounds::default();
        loop {
            self.parse_type_bound(&mut set)?;
            if !self.match_token(&TokenKind::Plus) {
                break;
            }
        }
        Ok(set)
    }

    pub(super) fn parse_where_clause(
        &mut self,
        type_params: &[String],
        bounds: &mut Vec<TypeBounds>,
    ) -> Result<()> {
        if !matches!(&self.peek().kind, TokenKind::Identifier(n) if n == "where") {
            return Ok(());
        }
        self.advance();
        loop {
            let span = self.peek().span;
            let name = self.consume_identifier("type parameter")?;
            self.consume(&TokenKind::Colon, ":")?;
            let set = self.parse_bound_list()?;
            let Some(i) = type_params.iter().position(|p| *p == name) else {
                return Err(self.rule(
                    106,
                    &format!("`where` names `{name}`, which is not a type parameter of this item"),
                    span,
                ));
            };
            bounds.resize(type_params.len(), TypeBounds::default());
            bounds[i].merge(&set);
            if !self.match_token(&TokenKind::Comma) || self.check(&TokenKind::LBrace) {
                break;
            }
        }
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
        self.function_item_with(decorators, is_pub, is_nogc, allow_receiver, false)
    }

    pub(super) fn function_item_with(
        &mut self,
        decorators: Vec<Decorator>,
        is_pub: bool,
        is_nogc: bool,
        allow_receiver: bool,
        body_optional: bool,
    ) -> Result<(Function, Receiver)> {
        let start_span = self.peek().span;
        self.advance();

        let name = self.consume_value_name("function name")?;

        let (type_params, bounds) = if self.match_token(&TokenKind::Lt) {
            let mut params = Vec::new();
            let mut bounds = Vec::new();
            loop {
                params.push(self.consume_identifier("type parameter")?);
                let set = if self.match_token(&TokenKind::Colon) {
                    self.parse_bound_list()?
                } else {
                    TypeBounds::default()
                };
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
        self.mut_self = false;
        if let Some((kind, len)) = self.receiver_ahead() {
            if !allow_receiver {
                return Err(self.error(CompileErrorKind::MethodSyntax {
                    message: Self::self_param_message(),
                }));
            }
            self.mut_self = self.peek_at(0).kind == TokenKind::Mut;
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

        let mut bounds = bounds;
        self.parse_where_clause(&type_params, &mut bounds)?;

        let body = if body_optional && !self.check(&TokenKind::LBrace) {
            self.bodiless = true;
            Vec::new()
        } else {
            self.consume(&TokenKind::LBrace, "{")?;
            self.block_statements()?
        };
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

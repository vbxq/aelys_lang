use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{
    ImplDecl, Method, Stmt, StmtKind, TokenKind, TraitDecl, TraitMethod, TraitRef, TypeBounds,
};

impl Parser {
    pub(super) fn impl_declaration(&mut self) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance();

        if self.block_depth > 0 {
            return Err(self.error(CompileErrorKind::MethodSyntax {
                message: "an `impl` block belongs at the top level of a module".to_string(),
            }));
        }

        let (type_params, mut bounds) = if self.match_token(&TokenKind::Lt) {
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
            self.consume_gt()?;
            (params, bounds)
        } else {
            (Vec::new(), Vec::new())
        };

        let trait_ref = if self.check(&TokenKind::Nogc) && self.peek_at(1).kind == TokenKind::For {
            let span = self.peek().span;
            self.advance();
            self.advance();
            Some(TraitRef::new("nogc".to_string(), span))
        } else {
            None
        };
        let first = self.parse_impl_head()?;
        let (trait_ref, self_ty) = if trait_ref.is_some() {
            (trait_ref, first)
        } else if self.match_token(&TokenKind::For) {
            if first.reference.is_some() || first.is_slice || first.is_function_type() {
                return Err(self.rule(
                    757,
                    "the name after `impl` and before `for` is a trait name",
                    first.span,
                ));
            }
            let mut args: Vec<_> = first.type_params.clone();
            if args.is_empty()
                && let Some(p) = &first.type_param
            {
                args.push((**p).clone());
            }
            let r = TraitRef {
                name: first.name.clone(),
                span: first.span,
                args,
            };
            (Some(r), self.parse_type_annotation()?)
        } else {
            (None, first)
        };
        self.parse_where_clause(&type_params, &mut bounds)?;

        self.consume(&TokenKind::LBrace, "{")?;

        let mut methods = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            if self.match_token(&TokenKind::Semicolon) {
                continue;
            }
            let decorators = self.decorators()?;
            let is_pub = self.match_token(&TokenKind::Pub);
            let is_nogc = self.match_token(&TokenKind::Nogc);
            if !self.check(&TokenKind::Fn) {
                return Err(self.error(CompileErrorKind::MethodSyntax {
                    message: format!(
                        "only `fn` items are allowed in an `impl` block, found {}",
                        self.peek().kind
                    ),
                }));
            }
            let (func, receiver) = self.function_item(decorators, is_pub, is_nogc, true)?;
            methods.push(Method {
                func,
                receiver,
                mut_self: self.mut_self,
            });
        }

        self.consume(&TokenKind::RBrace, "}")?;
        let span = start_span.merge(self.previous().span);

        Ok(Stmt::new(
            StmtKind::Impl(ImplDecl {
                type_params,
                bounds,
                trait_ref,
                self_ty,
                methods,
                span,
            }),
            span,
        ))
    }
}

impl Parser {
    fn parse_impl_head(&mut self) -> Result<aelys_syntax::TypeAnnotation> {
        if self.check(&TokenKind::LBracket) {
            let save = self.current;
            let start = self.peek().span;
            self.advance();
            let element = self.parse_type_annotation()?;
            if self.match_token(&TokenKind::RBracket) {
                let span = start.merge(self.previous().span);
                return Ok(aelys_syntax::TypeAnnotation::slice_referent(element, span));
            }
            self.current = save;
        }
        self.parse_type_annotation()
    }

    pub(super) fn starts_trait(&self) -> bool {
        matches!(&self.peek().kind, TokenKind::Identifier(n) if n == "trait")
            && matches!(self.peek_at(1).kind, TokenKind::Identifier(_))
            && matches!(self.peek_at(2).kind, TokenKind::LBrace | TokenKind::Lt)
    }

    pub(super) fn trait_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance();
        if self.block_depth > 0 {
            return Err(self.rule(
                757,
                "a `trait` belongs at the top level of a module",
                start_span,
            ));
        }
        let name = self.consume_identifier("trait name")?;
        let mut type_params = Vec::new();
        if self.match_token(&TokenKind::Lt) {
            loop {
                type_params.push(self.consume_identifier("type parameter")?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume_gt()?;
        }
        self.consume(&TokenKind::LBrace, "{")?;
        let mut methods = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            if self.match_token(&TokenKind::Semicolon) {
                continue;
            }
            let decorators = self.decorators()?;
            let is_pub = self.match_token(&TokenKind::Pub);
            let is_nogc = self.match_token(&TokenKind::Nogc);
            if !self.check(&TokenKind::Fn) {
                return Err(self.rule(
                    757,
                    &format!(
                        "only `fn` items are allowed in a `trait`, found {}",
                        self.peek().kind
                    ),
                    self.peek().span,
                ));
            }
            self.bodiless = false;
            let (func, receiver) =
                self.function_item_with(decorators, is_pub, is_nogc, true, true)?;
            let has_default = !self.bodiless;
            self.bodiless = false;
            methods.push(TraitMethod {
                func,
                receiver,
                has_default,
                mut_self: self.mut_self,
            });
        }
        self.consume(&TokenKind::RBrace, "}")?;
        let span = start_span.merge(self.previous().span);
        Ok(Stmt::new(
            StmtKind::Trait(TraitDecl {
                name,
                type_params,
                is_pub,
                methods,
                span,
            }),
            span,
        ))
    }
}

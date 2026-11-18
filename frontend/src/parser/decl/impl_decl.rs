use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{ImplDecl, Method, Stmt, StmtKind, TokenKind, TypeBounds};

impl Parser {
    pub(super) fn impl_declaration(&mut self) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance();

        if self.block_depth > 0 {
            return Err(self.error(CompileErrorKind::MethodSyntax {
                message: "an `impl` block belongs at the top level of a module".to_string(),
            }));
        }

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
            self.consume_gt()?;
            (params, bounds)
        } else {
            (Vec::new(), Vec::new())
        };

        let self_ty = self.parse_type_annotation()?;

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
            methods.push(Method { func, receiver });
        }

        self.consume(&TokenKind::RBrace, "}")?;
        let span = start_span.merge(self.previous().span);

        Ok(Stmt::new(
            StmtKind::Impl(ImplDecl {
                type_params,
                bounds,
                self_ty,
                methods,
                span,
            }),
            span,
        ))
    }
}

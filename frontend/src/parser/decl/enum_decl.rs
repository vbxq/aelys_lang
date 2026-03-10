use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{EnumVariantDecl, Stmt, StmtKind, TokenKind};

impl Parser {
    pub(super) fn enum_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance(); // consume `enum`

        let name = self.consume_identifier("enum name")?;

        if name.chars().next().is_none_or(|c| !c.is_uppercase()) {
            return Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: "capitalized enum name".to_string(),
                found: name,
            }));
        }

        let type_params = if self.match_token(&TokenKind::Lt) {
            let mut params = Vec::new();
            loop {
                params.push(self.consume_identifier("type parameter")?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume(&TokenKind::Gt, ">")?;
            params
        } else {
            Vec::new()
        };

        self.consume(&TokenKind::LBrace, "{")?;

        let mut variants = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let variant_span = self.peek().span;
            let variant_name = self.consume_identifier("variant name")?;

            if variant_name.chars().next().is_none_or(|c| !c.is_uppercase()) {
                return Err(self.error(CompileErrorKind::UnexpectedToken {
                    expected: "capitalized variant name".to_string(),
                    found: variant_name,
                }));
            }

            // Parse optional tuple fields: Variant(Type1, Type2, ...)
            let fields = if self.match_token(&TokenKind::LParen) {
                let mut field_types = Vec::new();
                if !self.check(&TokenKind::RParen) {
                    loop {
                        field_types.push(self.parse_type_annotation()?);
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.consume(&TokenKind::RParen, ")")?;
                field_types
            } else {
                Vec::new()
            };

            let end_span = self.previous().span;
            variants.push(EnumVariantDecl {
                name: variant_name,
                fields,
                span: variant_span.merge(end_span),
            });

            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }

        self.consume(&TokenKind::RBrace, "}")?;
        let end_span = self.previous().span;

        Ok(Stmt::new(
            StmtKind::EnumDecl {
                name,
                type_params,
                variants,
                is_pub,
            },
            start_span.merge(end_span),
        ))
    }
}

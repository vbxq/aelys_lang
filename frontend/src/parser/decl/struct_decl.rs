use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{Stmt, StmtKind, StructFieldDecl, TokenKind};

impl Parser {
    pub(super) fn struct_declaration(&mut self, is_pub: bool) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance(); // consume `struct`

        let name = self.consume_identifier("struct name")?;

        if name.chars().next().map_or(true, |c| !c.is_uppercase()) {
            return Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: "capitalized struct name".to_string(),
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

        let mut fields = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let field_span = self.peek().span;
            let field_name = self.consume_identifier("field name")?;
            self.consume(&TokenKind::Colon, ":")?;
            let type_annotation = self.parse_type_annotation()?;
            let end_span = self.previous().span;

            fields.push(StructFieldDecl {
                name: field_name,
                type_annotation,
                span: field_span.merge(end_span),
            });

            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }

        self.consume(&TokenKind::RBrace, "}")?;
        let end_span = self.previous().span;

        Ok(Stmt::new(
            StmtKind::StructDecl {
                name,
                type_params,
                fields,
                is_pub,
            },
            start_span.merge(end_span),
        ))
    }
}

use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Parameter, TokenKind, TypeAnnotation};

impl Parser {
    pub fn parse_type_annotation(&mut self) -> Result<TypeAnnotation> {
        let start_span = self.peek().span;
        let name = self.consume_identifier("type name")?;

        // this checks for generic type parameter: array<int>, vec<string>, etc.
        if self.match_token(&TokenKind::Lt) {
            let type_param = self.parse_type_annotation()?;
            self.consume(&TokenKind::Gt, ">")?;
            let end_span = self.previous().span;
            Ok(TypeAnnotation::with_param(
                name,
                type_param,
                start_span.merge(end_span),
            ))
        } else {
            Ok(TypeAnnotation::new(name, start_span))
        }
    }

    pub fn parse_parameter(&mut self) -> Result<Parameter> {
        let span = self.peek().span;
        let mutable = self.match_token(&TokenKind::Mut);
        let name = self.consume_identifier("parameter name")?;

        let type_annotation = if self.match_token(&TokenKind::Colon) {
            Some(self.parse_type_annotation()?)
        } else {
            None
        };

        let end_span = self.previous().span;
        Ok(Parameter::new(
            name,
            mutable,
            type_annotation,
            span.merge(end_span),
        ))
    }
}

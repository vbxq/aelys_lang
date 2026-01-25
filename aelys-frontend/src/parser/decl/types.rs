use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Parameter, TokenKind, TypeAnnotation};

impl Parser {
    pub fn parse_type_annotation(&mut self) -> Result<TypeAnnotation> {
        let span = self.peek().span;
        let name = self.consume_identifier("type name")?;
        Ok(TypeAnnotation::new(name, span))
    }

    pub fn parse_parameter(&mut self) -> Result<Parameter> {
        let span = self.peek().span;
        let name = self.consume_identifier("parameter name")?;

        let type_annotation = if self.match_token(&TokenKind::Colon) {
            Some(self.parse_type_annotation()?)
        } else {
            None
        };

        let end_span = self.previous().span;
        Ok(Parameter::new(name, type_annotation, span.merge(end_span)))
    }
}

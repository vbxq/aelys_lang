use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Parameter, TokenKind, TypeAnnotation};

impl Parser {
    pub fn parse_type_annotation(&mut self) -> Result<TypeAnnotation> {
        let start_span = self.peek().span;

        if self.match_token(&TokenKind::LBracket) {
            let inner = self.parse_type_annotation()?;
            self.consume(&TokenKind::Semicolon, ";")?;
            let size_token = self.advance();
            let size = match &size_token.kind {
                TokenKind::Int(n) if *n >= 0 => *n as u64,
                other => {
                    let found = other.to_string();
                    return Err(self.error(
                        aelys_common::error::CompileErrorKind::UnexpectedToken {
                            expected: "positive integer for array size".to_string(),
                            found,
                        },
                    ));
                }
            };
            self.consume(&TokenKind::RBracket, "]")?;
            let end_span = self.previous().span;
            return Ok(TypeAnnotation::array_sized(
                inner,
                size,
                start_span.merge(end_span),
            ));
        }

        if self.match_token(&TokenKind::Fn) {
            return self.parse_function_type_annotation(start_span);
        }

        let name = self.consume_identifier("type name")?;

        if self.match_token(&TokenKind::Lt) {
            let mut type_params = Vec::new();
            type_params.push(self.parse_type_annotation()?);
            while self.match_token(&TokenKind::Comma) {
                type_params.push(self.parse_type_annotation()?);
            }
            self.consume_gt()?;
            let end_span = self.previous().span;
            if type_params.len() == 1 {
                // SAFETY: length was just checked to be exactly 1.
                let single = type_params.into_iter().next().expect("len == 1");
                Ok(TypeAnnotation::with_param(
                    name,
                    single,
                    start_span.merge(end_span),
                ))
            } else {
                Ok(TypeAnnotation::with_params(
                    name,
                    type_params,
                    start_span.merge(end_span),
                ))
            }
        } else {
            Ok(TypeAnnotation::new(name, start_span))
        }
    }

    fn parse_function_type_annotation(
        &mut self,
        start_span: aelys_syntax::Span,
    ) -> Result<TypeAnnotation> {
        self.consume(&TokenKind::LParen, "(")?;
        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            params.push(self.parse_type_annotation()?);
            while self.match_token(&TokenKind::Comma) {
                params.push(self.parse_type_annotation()?);
            }
        }
        self.consume(&TokenKind::RParen, ")")?;
        self.consume(&TokenKind::Arrow, "->")?;
        let ret = self.parse_type_annotation()?;
        let end_span = self.previous().span;
        Ok(TypeAnnotation::function_type(
            params,
            ret,
            start_span.merge(end_span),
        ))
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

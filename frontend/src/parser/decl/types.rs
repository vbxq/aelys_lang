use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Parameter, RefKind, TokenKind, TypeAnnotation};

impl Parser {
    pub fn parse_type_annotation(&mut self) -> Result<TypeAnnotation> {
        let start_span = self.peek().span;

        if self.match_token(&TokenKind::Ampersand) {
            let kind = if self.match_token(&TokenKind::Mut) {
                RefKind::Mut
            } else {
                RefKind::Shared
            };
            if self.match_token(&TokenKind::LBracket) {
                let element = self.parse_type_annotation()?;
                self.consume(&TokenKind::RBracket, "]")?;
                let end_span = self.previous().span;
                let mut ann = TypeAnnotation::slice_referent(element, start_span.merge(end_span));
                ann.reference = Some(kind);
                return Ok(ann);
            }
            let mut referent = self.parse_type_annotation()?;
            referent.reference = Some(kind);
            referent.span = start_span.merge(referent.span);
            return Ok(referent);
        }

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

        let nogc = self.match_token(&TokenKind::Nogc);
        if self.match_token(&TokenKind::Fn) {
            return self.parse_function_type_annotation(start_span, nogc);
        }
        if nogc {
            return Err(
                self.error(aelys_common::error::CompileErrorKind::UnexpectedToken {
                    expected: "`fn` after `nogc` in a type".to_string(),
                    found: self.peek().kind.to_string(),
                }),
            );
        }

        let mut name = self.consume_identifier("type name")?;

        // a module-qualified type is one name, and `.` is not an identifier character
        while self.check(&TokenKind::Dot)
            && matches!(self.peek_at(1).kind, TokenKind::Identifier(_))
        {
            self.advance();
            let segment = self.consume_identifier("type name segment")?;
            name.push('.');
            name.push_str(&segment);
        }

        if self.match_token(&TokenKind::Lt) {
            let mut type_params = Vec::new();
            type_params.push(self.parse_type_annotation()?);
            while self.match_token(&TokenKind::Comma) {
                type_params.push(self.parse_type_annotation()?);
            }
            self.consume_gt()?;
            let end_span = self.previous().span;
            if type_params.len() == 1 {
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
        nogc: bool,
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
        let (ret, end_span) = if self.match_token(&TokenKind::Arrow) {
            let ret = self.parse_type_annotation()?;
            (ret, self.previous().span)
        } else {
            let end = self.previous().span;
            (TypeAnnotation::new("null".to_string(), end), end)
        };
        Ok(TypeAnnotation::function_type(
            params,
            ret,
            nogc,
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

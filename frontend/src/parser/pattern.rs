use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{PatLit, Pattern, Span, TokenKind};

impl Parser {
    pub(super) fn parse_let_pattern(&mut self) -> Result<Pattern> {
        let outer = std::mem::replace(&mut self.pattern_in_let, true);
        let result = self.parse_pattern();
        self.pattern_in_let = outer;
        result
    }

    pub(super) fn parse_pattern(&mut self) -> Result<Pattern> {
        self.enter_recursion()?;
        let result = self.parse_or_pattern();
        self.exit_recursion();
        result
    }

    fn parse_or_pattern(&mut self) -> Result<Pattern> {
        self.match_token(&TokenKind::Pipe);
        let start = self.peek().span;
        let first = self.parse_at_pattern()?;
        if !self.check(&TokenKind::Pipe) {
            return Ok(first);
        }
        let mut alts = vec![first];
        while self.match_token(&TokenKind::Pipe) {
            alts.push(self.parse_at_pattern()?);
        }
        let end = self.previous().span;
        Ok(Pattern::Or(alts, start.merge(end)))
    }

    fn parse_at_pattern(&mut self) -> Result<Pattern> {
        let start = self.peek().span;
        if let TokenKind::Identifier(name) = &self.peek().kind
            && name != "_"
            && matches!(self.peek_at(1).kind, TokenKind::At)
        {
            let name = self.consume_binding_name("binding name")?;
            self.advance();
            let sub = self.parse_at_pattern()?;
            let end = self.previous().span;
            return Ok(Pattern::Binding {
                name,
                mutable: false,
                sub: Some(Box::new(sub)),
                span: start.merge(end),
            });
        }
        self.parse_primary_pattern()
    }

    fn parse_primary_pattern(&mut self) -> Result<Pattern> {
        let span = self.peek().span;
        match self.peek().kind.clone() {
            TokenKind::Identifier(name) if name == "_" => {
                self.advance();
                Ok(Pattern::Wildcard(span))
            }
            TokenKind::Minus
            | TokenKind::Int(_)
            | TokenKind::UInt(_)
            | TokenKind::SuffixedInt(..)
            | TokenKind::Byte(_)
            | TokenKind::Float(_)
            | TokenKind::Char(_)
            | TokenKind::String(_)
            | TokenKind::True
            | TokenKind::False => self.parse_literal_pattern(),
            TokenKind::Ampersand | TokenKind::AndAnd => self.fail_help(
                136,
                "reference patterns are not supported; values are copied, bind `x` directly",
                "write the binding without `&`",
                span,
            ),
            TokenKind::DotDot | TokenKind::DotDotEq => {
                self.advance();
                if self.pattern_literal_follows() {
                    self.parse_pat_lit()?;
                }
                self.refuse_pattern(
                    "a range pattern without a lower bound is not supported; write `lo..=hi`",
                    span,
                );
                Ok(Pattern::Wildcard(span))
            }
            TokenKind::LParen => {
                self.advance();
                if self.match_token(&TokenKind::RParen) {
                    let end = self.previous().span;
                    return self.fail(
                        102,
                        "expected a pattern, found `()`: there is no unit value",
                        span.merge(end),
                    );
                }
                let first = self.parse_pattern()?;
                if !self.check(&TokenKind::Comma) {
                    self.consume(&TokenKind::RParen, ")")?;
                    if self.pattern_in_let {
                        let end = self.previous().span;
                        return self.fail(
                            902,
                            "a one-element tuple pattern is not supported; bind the value itself",
                            span.merge(end),
                        );
                    }
                    return Ok(first);
                }
                let mut elems = vec![first];
                while self.match_token(&TokenKind::Comma) {
                    if self.check(&TokenKind::RParen) {
                        break;
                    }
                    elems.push(self.parse_pattern()?);
                }
                self.consume(&TokenKind::RParen, ")")?;
                let end = self.previous().span;
                if elems.len() == 1 {
                    return self.fail(
                        902,
                        "a one-element tuple pattern is not supported; bind the value itself",
                        span.merge(end),
                    );
                }
                Ok(Pattern::Tuple {
                    elems,
                    span: span.merge(end),
                })
            }
            TokenKind::Mut => {
                self.advance();
                let name = self.consume_binding_name("binding name")?;
                let end = self.previous().span;
                if !self.pattern_in_let {
                    self.refuse_pattern(
                        "a `mut` binding is not supported in a match pattern; bind the value, \
                         then write `let mut` in the arm",
                        span.merge(end),
                    );
                }
                Ok(Pattern::Binding {
                    name,
                    mutable: true,
                    sub: None,
                    span: span.merge(end),
                })
            }
            TokenKind::LBracket => {
                self.refuse_in_place(
                    CompileErrorKind::PatternFormNotSupported {
                        what: "slice and array".to_string(),
                    },
                    span,
                );
                self.skip_balanced(TokenKind::LBracket, TokenKind::RBracket);
                Ok(Pattern::Wildcard(span))
            }
            TokenKind::Ref => {
                self.refuse_in_place(
                    CompileErrorKind::PatternFormNotSupported {
                        what: "`ref`".to_string(),
                    },
                    span,
                );
                self.advance();
                if matches!(&self.peek().kind, TokenKind::Mut) {
                    self.advance();
                }
                self.parse_primary_pattern()
            }
            TokenKind::Identifier(_) | TokenKind::Crate => {
                let lo = self.parse_path_pattern()?;
                if is_const_reference(&lo) && self.match_token(&TokenKind::DotDotEq) {
                    let hi = self.parse_range_bound()?;
                    let span = span.merge(self.previous().span);
                    return Ok(Pattern::ConstRange {
                        lo: Box::new(lo),
                        hi: Box::new(hi),
                        span,
                    });
                }
                Ok(lo)
            }
            _ => Err(self.name_error("a pattern")),
        }
    }

    fn parse_path_pattern(&mut self) -> Result<Pattern> {
        let span = self.peek().span;
        if self.match_token(&TokenKind::Crate) {
            self.consume(&TokenKind::ColonColon, "`::` after `crate`")?;
        }
        let first = self.consume_identifier("a pattern")?;
        if self.check(&TokenKind::Dot) && matches!(self.peek_at(1).kind, TokenKind::Identifier(_)) {
            let mut path = first.clone();
            let mut i = 0;
            while matches!(self.peek_at(i).kind, TokenKind::Dot | TokenKind::ColonColon) {
                if let TokenKind::Identifier(s) = &self.peek_at(i + 1).kind {
                    path.push_str("::");
                    path.push_str(s);
                }
                i += 2;
            }
            return self.fail(115, &format!("a module path is written with `::`: write `{path}`"), span);
        }
        let mut segments = vec![first];
        while self.check(&TokenKind::ColonColon) && matches!(self.peek_at(1).kind, TokenKind::Identifier(_)) {
            self.advance();
            segments.push(self.consume_identifier("a path segment")?);
        }
        let next = self.peek().kind.clone();
        if segments.len() > 1 {
            let variant = segments.pop().unwrap();
            let enum_name = segments
                .iter()
                .map(|s| super::types::canonical_type_name(s).to_string())
                .collect::<Vec<_>>()
                .join(".");
            if matches!(next, TokenKind::LBrace) {
                return self.parse_struct_pattern(format!("{enum_name}.{variant}"), span);
            }
            let fields = self.variant_fields()?;
            return Ok(Pattern::Variant {
                enum_name,
                variant,
                fields,
                span: span.merge(self.previous().span),
                unqualified: false,
            });
        }
        let first = segments.pop().unwrap();
        let lower = !first.starts_with(char::is_uppercase);
        if (lower || self.pattern_in_let) && !matches!(next, TokenKind::LParen | TokenKind::LBrace) {
            if !self.refuse_reserved_value_name(&first, span) && !lower {
                self.refuse_in_place(CompileErrorKind::UppercaseBinding { name: first.clone() }, span);
            }
            return Ok(Pattern::Binding {
                name: first,
                mutable: false,
                sub: None,
                span,
            });
        }
        if lower && matches!(next, TokenKind::LParen) {
            self.refuse_in_place(CompileErrorKind::NotAVariantInScope { name: first }, span);
            self.skip_balanced(TokenKind::LParen, TokenKind::RParen);
            return Ok(Pattern::Wildcard(span));
        }
        if matches!(next, TokenKind::LBrace) {
            if lower {
                return self.unexpected("`=>`");
            }
            return self.parse_struct_pattern(first, span);
        }
        let enum_name = aelys_syntax::prelude_variant_enum(&first).unwrap_or("").to_string();
        let fields = self.variant_fields()?;
        Ok(Pattern::Variant {
            enum_name,
            variant: first,
            fields,
            span: span.merge(self.previous().span),
            unqualified: true,
        })
    }

    fn variant_fields(&mut self) -> Result<Vec<Pattern>> {
        let mut fields = Vec::new();
        if self.match_token(&TokenKind::LParen) {
            while !self.check(&TokenKind::RParen) {
                fields.push(self.parse_pattern()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
            self.consume(&TokenKind::RParen, "`)`")?;
        }
        Ok(fields)
    }

    fn parse_struct_pattern(&mut self, name: String, span: Span) -> Result<Pattern> {
        self.consume(&TokenKind::LBrace, "{")?;
        let mut fields = Vec::new();
        let mut rest = false;
        while !self.check(&TokenKind::RBrace) {
            if self.match_token(&TokenKind::DotDot) {
                rest = true;
                break;
            }
            let field_span = self.peek().span;
            let field = self.consume_member_name("field name")?;
            let pat = if self.match_token(&TokenKind::Colon) {
                self.parse_pattern()?
            } else {
                if field.starts_with(char::is_uppercase) {
                    self.refuse_in_place(
                        CompileErrorKind::UppercaseBinding {
                            name: field.clone(),
                        },
                        field_span,
                    );
                } else {
                    self.refuse_reserved_value_name(&field, field_span);
                }
                Pattern::Binding {
                    name: field.clone(),
                    mutable: false,
                    sub: None,
                    span: field_span,
                }
            };
            let end = self.previous().span;
            fields.push((field, pat, field_span.merge(end)));
            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }
        self.consume(&TokenKind::RBrace, "}")?;
        let end = self.previous().span;
        Ok(Pattern::Struct {
            name,
            fields,
            rest,
            span: span.merge(end),
        })
    }

    fn pattern_literal_follows(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::Minus
                | TokenKind::Int(_)
                | TokenKind::UInt(_)
                | TokenKind::SuffixedInt(..)
                | TokenKind::Byte(_)
                | TokenKind::Float(_)
                | TokenKind::Char(_)
                | TokenKind::String(_)
                | TokenKind::True
                | TokenKind::False
        )
    }

    fn parse_pat_lit(&mut self) -> Result<Option<PatLit>> {
        let span = self.peek().span;
        let negative = self.match_token(&TokenKind::Minus);
        let tok = self.peek().kind.clone();
        let lit = match tok {
            TokenKind::Int(v) => PatLit::Int(v as i128),
            TokenKind::UInt(v) => PatLit::Int(v as i128),
            TokenKind::SuffixedInt(v, _) => PatLit::Int(v as i128),
            TokenKind::Byte(b) if !negative => PatLit::Int(b as i128),
            TokenKind::Float(_) => {
                self.advance();
                self.refuse_pattern(
                    "float literal patterns are not supported; use a guard such as `x if x == 1.5`",
                    span,
                );
                return Ok(None);
            }
            TokenKind::Char(c) if !negative => PatLit::Char(c),
            TokenKind::String(s) if !negative => PatLit::Str(s),
            TokenKind::True if !negative => PatLit::Bool(true),
            TokenKind::False if !negative => PatLit::Bool(false),
            other => {
                return Err(self.error(CompileErrorKind::UnexpectedToken {
                    expected: "literal".to_string(),
                    found: other.to_string(),
                }));
            }
        };
        self.advance();
        Ok(Some(match lit {
            PatLit::Int(v) if negative => PatLit::Int(-v),
            other => other,
        }))
    }

    fn parse_range_bound(&mut self) -> Result<Pattern> {
        let span = self.peek().span;
        if matches!(self.peek().kind, TokenKind::Identifier(_)) {
            let bound = self.parse_path_pattern()?;
            if !is_const_reference(&bound) {
                self.refuse_pattern(
                    "a range bound is a literal or a const; write `lo..=hi`",
                    bound.span(),
                );
                return Ok(Pattern::Wildcard(bound.span()));
            }
            return Ok(bound);
        }
        let lit = self.parse_pat_lit()?;
        let span = span.merge(self.previous().span);
        Ok(match lit {
            Some(lit) => Pattern::Lit(lit, span),
            None => Pattern::Wildcard(span),
        })
    }

    fn parse_literal_pattern(&mut self) -> Result<Pattern> {
        let span = self.peek().span;
        let lo = self.parse_pat_lit()?;
        if self.check(&TokenKind::DotDotEq)
            && matches!(self.peek_at(1).kind, TokenKind::Identifier(_))
        {
            self.advance();
            let lo_span = span.merge(self.previous().span);
            let hi = self.parse_range_bound()?;
            let span = span.merge(self.previous().span);
            let Some(lo) = lo else {
                return Ok(Pattern::Wildcard(span));
            };
            return Ok(Pattern::ConstRange {
                lo: Box::new(Pattern::Lit(lo, lo_span)),
                hi: Box::new(hi),
                span,
            });
        }
        if self.match_token(&TokenKind::DotDotEq) {
            let hi = self.parse_pat_lit()?;
            let end = self.previous().span;
            let span = span.merge(end);
            let (Some(lo), Some(hi)) = (lo, hi) else {
                return Ok(Pattern::Wildcard(span));
            };
            let empty = match (&lo, &hi) {
                (PatLit::Int(a), PatLit::Int(b)) => a > b,
                (PatLit::Char(a), PatLit::Char(b)) => a > b,
                _ => false,
            };
            if empty {
                self.refuse_pattern(
                    &format!(
                        "empty range pattern `{}..={}`: no value lies between these bounds; \
                         write the smaller bound first",
                        show_lit(&lo),
                        show_lit(&hi)
                    ),
                    span,
                );
                return Ok(Pattern::Wildcard(span));
            }
            return Ok(Pattern::Range { lo, hi, span });
        }
        if self.match_token(&TokenKind::DotDot) {
            let hi = if self.pattern_literal_follows() {
                self.parse_pat_lit()?
            } else {
                None
            };
            let end = self.previous().span;
            let span = span.merge(end);
            let detail = match (&lo, &hi) {
                (Some(PatLit::Int(a)), Some(PatLit::Int(b))) => format!(
                    "exclusive range pattern `{a}..{b}` is not supported; write `{a}..={}`",
                    b - 1
                ),
                (Some(lo), Some(hi)) => format!(
                    "exclusive range pattern `{}..{}` is not supported; write an inclusive range \
                     `lo..=hi`",
                    show_lit(lo),
                    show_lit(hi)
                ),
                _ => "half-open range patterns are not supported; write an inclusive range \
                      `lo..=hi`"
                    .to_string(),
            };
            self.refuse_pattern(&detail, span);
            return Ok(Pattern::Wildcard(span));
        }
        let end = self.previous().span;
        match lo {
            Some(lit) => Ok(Pattern::Lit(lit, span.merge(end))),
            None => Ok(Pattern::Wildcard(span)),
        }
    }

    pub(super) fn refuse_pattern(&mut self, detail: &str, span: Span) {
        self.refuse_in_place(
            CompileErrorKind::PatternUnsupported {
                detail: detail.to_string(),
            },
            span,
        );
    }

    fn skip_balanced(&mut self, open: TokenKind, close: TokenKind) {
        if self.check(&open) {
            self.advance();
        }
        let mut depth = 1usize;
        while !self.is_at_end() {
            let kind = self.advance().kind.clone();
            if kind == open {
                depth += 1;
            } else if kind == close {
                depth -= 1;
                if depth == 0 {
                    return;
                }
            }
        }
    }
}

fn show_lit(lit: &PatLit) -> String {
    match lit {
        PatLit::Int(v) => v.to_string(),
        PatLit::Char(c) => match char::from_u32(*c) {
            Some(ch) if !ch.is_control() => format!("'{ch}'"),
            _ => format!("'\\u{{{c:X}}}'"),
        },
        PatLit::Bool(b) => b.to_string(),
        PatLit::Str(s) => format!("{s:?}"),
    }
}

fn is_const_reference(p: &Pattern) -> bool {
    match p {
        Pattern::Variant {
            enum_name,
            variant,
            fields,
            unqualified,
            ..
        } => {
            fields.is_empty()
                && variant.starts_with(char::is_uppercase)
                && (*unqualified && enum_name.is_empty()
                    || !*unqualified && !enum_name.starts_with(char::is_uppercase))
        }
        _ => false,
    }
}

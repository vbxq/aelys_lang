use super::Parser;
use aelys_common::Result;
use aelys_syntax::{BinaryOp, Expr, ExprKind, TokenKind};

impl Parser {
    // calls and member access (highest precedence after atoms)
    pub(super) fn call(&mut self) -> Result<Expr> {
        let mut expr = self.primary()?;

        loop {
            if self.match_token(&TokenKind::LParen) {
                let mut args = Vec::new();

                if !self.check(&TokenKind::RParen) {
                    loop {
                        args.push(self.expression()?);
                        if !self.match_token(&TokenKind::Comma) {
                            break;
                        }
                    }
                }

                self.consume(&TokenKind::RParen, ")")?;
                let span = expr.span.merge(self.previous().span);

                expr = Expr::new(
                    ExprKind::Call {
                        callee: Box::new(expr),
                        args,
                    },
                    span,
                );
            } else if self.match_token(&TokenKind::Dot) {
                let member = self.consume_identifier("member name")?;
                let span = expr.span.merge(self.previous().span);

                expr = Expr::new(
                    ExprKind::Member {
                        object: Box::new(expr),
                        member,
                    },
                    span,
                );
            } else if self.match_token(&TokenKind::LBracket) {
                let index_or_range = self.parse_index_or_range()?;
                self.consume(&TokenKind::RBracket, "]")?;
                let span = expr.span.merge(self.previous().span);

                if matches!(index_or_range.kind, ExprKind::Range { .. }) {
                    expr = Expr::new(
                        ExprKind::Slice {
                            object: Box::new(expr),
                            range: Box::new(index_or_range),
                        },
                        span,
                    );
                } else {
                    expr = Expr::new(
                        ExprKind::Index {
                            object: Box::new(expr),
                            index: Box::new(index_or_range),
                        },
                        span,
                    );
                }
            } else if self.check(&TokenKind::PlusPlus) || self.check(&TokenKind::MinusMinus) {
                // on désucre x++ → x = x + 1, x-- → x = x - 1
                let op = if self.match_token(&TokenKind::PlusPlus) {
                    BinaryOp::Add
                } else {
                    self.advance(); // consume MinusMinus
                    BinaryOp::Sub
                };
                let span = expr.span.merge(self.previous().span);

                if let ExprKind::Identifier(ref name) = expr.kind {
                    let one = Expr::new(ExprKind::Int(1), self.previous().span);
                    let binary = Expr::new(
                        ExprKind::Binary {
                            left: Box::new(expr.clone()),
                            op,
                            right: Box::new(one),
                        },
                        span,
                    );
                    expr = Expr::new(
                        ExprKind::Assign {
                            name: name.clone(),
                            value: Box::new(binary),
                        },
                        span,
                    );
                } else {
                    break;
                }
            } else {
                break;
            }
        }

        Ok(expr)
    }

    /// Parse index or range expression inside brackets.
    /// Handles: arr[i], arr[1..3], arr[1..], arr[..3], arr[1..=3]
    fn parse_index_or_range(&mut self) -> Result<Expr> {
        let start_span = self.peek().span;

        // Check if range starts with .. or ..= (no start expression)
        if self.check(&TokenKind::DotDot) || self.check(&TokenKind::DotDotEq) {
            let inclusive = self.match_token(&TokenKind::DotDotEq);
            if !inclusive {
                self.advance(); // consume DotDot
            }

            // Check for end expression
            let end = if !self.check(&TokenKind::RBracket) {
                Some(Box::new(self.expression()?))
            } else {
                None
            };

            let end_span = self.previous().span;
            return Ok(Expr::new(
                ExprKind::Range {
                    start: None,
                    end,
                    inclusive,
                },
                start_span.merge(end_span),
            ));
        }

        // Parse the first expression (could be index or start of range)
        let first = self.expression()?;

        // Check if this is a range
        if self.check(&TokenKind::DotDot) || self.check(&TokenKind::DotDotEq) {
            let inclusive = self.match_token(&TokenKind::DotDotEq);
            if !inclusive {
                self.advance(); // consume DotDot
            }

            // Check for end expression
            let end = if !self.check(&TokenKind::RBracket) {
                Some(Box::new(self.expression()?))
            } else {
                None
            };

            let end_span = self.previous().span;
            return Ok(Expr::new(
                ExprKind::Range {
                    start: Some(Box::new(first)),
                    end,
                    inclusive,
                },
                start_span.merge(end_span),
            ));
        }

        // Just a simple index expression
        Ok(first)
    }
}

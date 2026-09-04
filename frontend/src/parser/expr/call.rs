use super::Parser;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::{BinaryOp, CatchHandler, Expr, ExprKind, TokenKind};

impl Parser {
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
                        if self.check(&TokenKind::RParen) {
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

                if let ExprKind::Identifier(namespace) = &expr.kind
                    && self.starts_a_qualified_struct_literal(&member)
                {
                    let qualified = format!("{}.{}", namespace, member);
                    expr = self.struct_literal(qualified, span)?;
                    continue;
                }

                expr = Expr::new(
                    ExprKind::Member {
                        object: Box::new(expr),
                        member,
                    },
                    span,
                );
            } else if self.match_token(&TokenKind::ColonColon) {
                let variant = self.consume_path_segment("variant name")?;
                let span = expr.span.merge(self.previous().span);

                let path = match &expr.kind {
                    ExprKind::Identifier(name) => Some(name.clone()),
                    ExprKind::Member { object, member }
                        if matches!(object.kind, ExprKind::Identifier(_)) =>
                    {
                        match &object.kind {
                            ExprKind::Identifier(namespace) => {
                                Some(format!("{}.{}", namespace, member))
                            }
                            _ => None,
                        }
                    }
                    _ => None,
                };

                match &path {
                    Some(enum_name) => {
                        let args = if self.check(&TokenKind::LParen) {
                            self.advance(); // consume '('
                            let mut args = Vec::new();
                            if !self.check(&TokenKind::RParen) {
                                loop {
                                    args.push(self.expression()?);
                                    if !self.match_token(&TokenKind::Comma) {
                                        break;
                                    }
                                    if self.check(&TokenKind::RParen) {
                                        break;
                                    }
                                }
                            }
                            self.consume(&TokenKind::RParen, ")")?;
                            let new_span = expr.span.merge(self.previous().span);
                            expr = Expr::new(
                                ExprKind::EnumVariant {
                                    enum_name: enum_name.clone(),
                                    variant,
                                    args,
                                },
                                new_span,
                            );
                            continue; // re-enter the loop for possible chaining
                        } else {
                            Vec::new()
                        };
                        expr = Expr::new(
                            ExprKind::EnumVariant {
                                enum_name: enum_name.clone(),
                                variant,
                                args,
                            },
                            span,
                        );
                    }
                    None => {
                        return Err(self.error(CompileErrorKind::UnexpectedToken {
                            expected: "enum name before ::".to_string(),
                            found: format!("{:?}", expr.kind),
                        }));
                    }
                }
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
                let op = if self.match_token(&TokenKind::PlusPlus) {
                    BinaryOp::Add
                } else {
                    self.advance(); // consume MinusMinus
                    BinaryOp::Sub
                };
                let span = expr.span.merge(self.previous().span);
                expr = Self::unwrap_grouping(expr);

                let one = Expr::new(ExprKind::Int(1), self.previous().span);
                if let ExprKind::Identifier(ref name) = expr.kind {
                    let binary = Expr::new(
                        ExprKind::Binary {
                            left: Box::new(expr.clone()),
                            op,
                            right: Box::new(one.clone()),
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
                } else if let ExprKind::Index {
                    ref object,
                    ref index,
                } = expr.kind
                {
                    let binary = Expr::new(
                        ExprKind::Binary {
                            left: Box::new(expr.clone()),
                            op,
                            right: Box::new(one.clone()),
                        },
                        span,
                    );
                    expr = Expr::new(
                        ExprKind::IndexAssign {
                            object: object.clone(),
                            index: index.clone(),
                            value: Box::new(binary),
                        },
                        span,
                    );
                } else if let ExprKind::Member {
                    ref object,
                    ref member,
                } = expr.kind
                {
                    let binary = Expr::new(
                        ExprKind::Binary {
                            left: Box::new(expr.clone()),
                            op,
                            right: Box::new(one),
                        },
                        span,
                    );
                    expr = Expr::new(
                        ExprKind::FieldAssign {
                            object: object.clone(),
                            field: member.clone(),
                            value: Box::new(binary),
                        },
                        span,
                    );
                } else if let ExprKind::Deref(ref target) = expr.kind {
                    // unconsumed and deleted the statement, silently
                    let binary = Expr::new(
                        ExprKind::Binary {
                            left: Box::new(expr.clone()),
                            op,
                            right: Box::new(one),
                        },
                        span,
                    );
                    expr = Expr::new(
                        ExprKind::DerefAssign {
                            target: target.clone(),
                            value: Box::new(binary),
                        },
                        span,
                    );
                } else {
                    return Err(CompileError::new(
                        CompileErrorKind::InvalidAssignmentTarget,
                        span,
                        std::sync::Arc::clone(&self.source),
                    )
                    .into());
                }
            } else if self.match_token(&TokenKind::As) {
                let target = self.parse_type_annotation()?;
                let span = expr.span.merge(self.previous().span);
                expr = Expr::new(
                    ExprKind::Cast {
                        expr: Box::new(expr),
                        target,
                    },
                    span,
                );
            } else if self.match_token(&TokenKind::Question) {
                let span = expr.span.merge(self.previous().span);
                expr = Expr::new(ExprKind::Try(Box::new(expr)), span);
            } else if self.match_token(&TokenKind::Catch) {
                let handler = if self.match_token(&TokenKind::Pipe) {
                    let name = self.consume_identifier("catch error binder")?;
                    self.consume(&TokenKind::Pipe, "|")?;
                    CatchHandler::Binding {
                        name,
                        body: Box::new(self.expression()?),
                    }
                } else if self.match_token(&TokenKind::LBrace) {
                    CatchHandler::Arms(self.match_arms()?)
                } else {
                    return Err(self.error(CompileErrorKind::UnexpectedToken {
                        expected: "`|` binder or `{` after catch".to_string(),
                        found: format!("{}", self.peek().kind),
                    }));
                };
                let span = expr.span.merge(self.previous().span);
                expr = Expr::new(
                    ExprKind::Catch {
                        scrutinee: Box::new(expr),
                        handler,
                    },
                    span,
                );
            } else {
                break;
            }
        }

        Ok(expr)
    }

    fn parse_index_or_range(&mut self) -> Result<Expr> {
        let start_span = self.peek().span;

        if self.check(&TokenKind::DotDot) || self.check(&TokenKind::DotDotEq) {
            let inclusive = self.match_token(&TokenKind::DotDotEq);
            if !inclusive {
                self.advance(); // consume DotDot
            }

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

        let first = self.expression()?;

        if self.check(&TokenKind::DotDot) || self.check(&TokenKind::DotDotEq) {
            let inclusive = self.match_token(&TokenKind::DotDotEq);
            if !inclusive {
                self.advance(); // consume DotDot
            }

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

        Ok(first)
    }
}

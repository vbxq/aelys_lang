use super::Parser;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::{Expr, ExprKind, FmtPart, FmtStringPart, Source, Stmt, StmtKind, TokenKind};
use std::sync::Arc;
use crate::lexer::Lexer;

impl Parser {
    pub(super) fn lambda_expression(&mut self, start_span: aelys_syntax::Span) -> Result<Expr> {
        self.consume(&TokenKind::LParen, "(")?;

        let mut params = Vec::new();
        if !self.check(&TokenKind::RParen) {
            loop {
                params.push(self.parse_parameter()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
            }
        }
        self.consume(&TokenKind::RParen, ")")?;

        let return_type = if self.match_token(&TokenKind::Arrow) {
            Some(self.parse_type_annotation()?)
        } else {
            None
        };

        let body = if self.check(&TokenKind::LBrace) {
            self.advance();
            self.block_statements()?
        } else {
            let expr = self.expression()?;
            vec![Stmt::new(StmtKind::Expression(expr.clone()), expr.span)]
        };

        let end_span = self.previous().span;

        Ok(Expr::new(
            ExprKind::Lambda {
                params,
                return_type,
                body,
            },
            start_span.merge(end_span),
        ))
    }

    pub(super) fn if_expression(&mut self, start_span: aelys_syntax::Span) -> Result<Expr> {
        let condition = self.expression()?;
        self.consume(&TokenKind::LBrace, "{")?;

        let then_branch = self.block_expression()?;

        self.consume(&TokenKind::Else, "else")?;
        self.consume(&TokenKind::LBrace, "{")?;
        let else_branch = self.block_expression()?;

        let end_span = self.previous().span;

        Ok(Expr::new(
            ExprKind::If {
                condition: Box::new(condition),
                then_branch: Box::new(then_branch),
                else_branch: Box::new(else_branch),
            },
            start_span.merge(end_span),
        ))
    }

    // block expr: last expr is the value (like Rust)
    pub(super) fn block_expression(&mut self) -> Result<Expr> {
        let mut stmts = Vec::new();

        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            if self.is_expression_start() {
                let expr = self.expression()?;

                if self.check(&TokenKind::RBrace) {
                    self.consume(&TokenKind::RBrace, "}")?;

                    return Ok(expr);
                }

                self.consume_semicolon()?;
                let span = expr.span;
                stmts.push(Stmt::new(StmtKind::Expression(expr), span));
            } else {
                if self.match_token(&TokenKind::Semicolon) {
                    continue;
                }
                stmts.push(self.declaration()?);
            }
        }

        self.consume(&TokenKind::RBrace, "}")?;

        Ok(Expr::new(ExprKind::Null, self.previous().span))
    }

    pub(super) fn is_expression_start(&self) -> bool {
        matches!(
            self.peek().kind,
            TokenKind::Int(_)
                | TokenKind::Float(_)
                | TokenKind::String(_)
                | TokenKind::FmtString(_)
                | TokenKind::True
                | TokenKind::False
                | TokenKind::Null
                | TokenKind::Identifier(_)
                | TokenKind::LParen
                | TokenKind::LBracket
                | TokenKind::Minus
                | TokenKind::Not
                | TokenKind::If
                | TokenKind::Fn
        )
    }

    pub(super) fn primary(&mut self) -> Result<Expr> {
        let token = self.advance();
        let span = token.span;
        let token_kind = token.kind.clone();

        let kind = match token_kind {
            TokenKind::Int(n) => ExprKind::Int(n),
            TokenKind::Float(n) => ExprKind::Float(n),
            TokenKind::String(s) => ExprKind::String(s),
            TokenKind::FmtString(parts) => {
                return self.parse_fmt_string(parts, span);
            }
            TokenKind::True => ExprKind::Bool(true),
            TokenKind::False => ExprKind::Bool(false),
            TokenKind::Null => ExprKind::Null,
            TokenKind::Identifier(ref name)
                if name.eq_ignore_ascii_case("array") || name.eq_ignore_ascii_case("vec") =>
            {
                let name = name.clone();
                return self.typed_collection_literal(name, span);
            }
            TokenKind::Identifier(name) => ExprKind::Identifier(name),

            TokenKind::LBracket => {
                return self.array_literal(span);
            }

            TokenKind::LParen => {
                let inner = self.expression()?;
                self.consume(&TokenKind::RParen, ")")?;
                let end_span = self.previous().span;
                return Ok(Expr::new(
                    ExprKind::Grouping(Box::new(inner)),
                    span.merge(end_span),
                ));
            }

            TokenKind::If => {
                return self.if_expression(span);
            }

            TokenKind::Fn => {
                return self.lambda_expression(span);
            }

            _ => {
                return Err(CompileError::new(
                    CompileErrorKind::ExpectedExpression,
                    span,
                    Arc::clone(&self.source),
                )
                .into());
            }
        };

        Ok(Expr::new(kind, span))
    }

    /// Parse array literal: [1, 2, 3] or sized array: [; 10]
    fn array_literal(&mut self, start_span: aelys_syntax::Span) -> Result<Expr> {
        // Check for sized array syntax: [; size]
        if self.match_token(&TokenKind::Semicolon) {
            let size = self.expression()?;
            self.consume(&TokenKind::RBracket, "]")?;
            let end_span = self.previous().span;
            return Ok(Expr::new(
                ExprKind::ArraySized {
                    element_type: None,
                    size: Box::new(size),
                },
                start_span.merge(end_span),
            ));
        }

        let mut elements = Vec::new();

        if !self.check(&TokenKind::RBracket) {
            loop {
                elements.push(self.expression()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RBracket) {
                    break;
                }
            }
        }

        self.consume(&TokenKind::RBracket, "]")?;
        let end_span = self.previous().span;

        Ok(Expr::new(
            ExprKind::ArrayLiteral {
                element_type: None,
                elements,
            },
            start_span.merge(end_span),
        ))
    }

    /// Parse typed collection literal: Array<Int>[1, 2, 3] or Vec<Float>[1.0, 2.0]
    /// Also handles sized arrays: Array(10) or Array<int>(10)
    fn typed_collection_literal(
        &mut self,
        collection_name: String,
        start_span: aelys_syntax::Span,
    ) -> Result<Expr> {
        let element_type = if self.match_token(&TokenKind::Lt) {
            let type_ann = self.parse_type_annotation()?;
            self.consume(&TokenKind::Gt, ">")?;
            Some(type_ann)
        } else {
            None
        };

        // Check for sized array constructor: Array(10) or Array<int>(10)
        // Only valid for Array, not Vec (Vec will fail at "[" consumption)
        if collection_name.eq_ignore_ascii_case("array") && self.match_token(&TokenKind::LParen) {
            let size = self.expression()?;
            self.consume(&TokenKind::RParen, ")")?;
            let end_span = self.previous().span;

            return Ok(Expr::new(
                ExprKind::ArraySized {
                    element_type,
                    size: Box::new(size),
                },
                start_span.merge(end_span),
            ));
        }

        self.consume(&TokenKind::LBracket, "[")?;

        // Check for sized array syntax: Array[; 10] or Array<int>[; 10]
        if self.match_token(&TokenKind::Semicolon) {
            let size = self.expression()?;
            self.consume(&TokenKind::RBracket, "]")?;
            let end_span = self.previous().span;
            return Ok(Expr::new(
                ExprKind::ArraySized {
                    element_type,
                    size: Box::new(size),
                },
                start_span.merge(end_span),
            ));
        }

        let mut elements = Vec::new();
        if !self.check(&TokenKind::RBracket) {
            loop {
                elements.push(self.expression()?);
                if !self.match_token(&TokenKind::Comma) {
                    break;
                }
                if self.check(&TokenKind::RBracket) {
                    break;
                }
            }
        }

        self.consume(&TokenKind::RBracket, "]")?;
        let end_span = self.previous().span;

        let kind = if collection_name.eq_ignore_ascii_case("vec") {
            ExprKind::VecLiteral {
                element_type,
                elements,
            }
        } else {
            ExprKind::ArrayLiteral {
                element_type,
                elements,
            }
        };

        Ok(Expr::new(kind, start_span.merge(end_span)))
    }

    fn parse_fmt_string(&mut self, parts: Vec<FmtPart>, span: aelys_syntax::Span) -> Result<Expr> {
        let mut result = Vec::new();

        for part in parts {
            match part {
                FmtPart::Literal(s) => result.push(FmtStringPart::Literal(s)),
                FmtPart::Placeholder => result.push(FmtStringPart::Placeholder),
                FmtPart::Expr(expr_str) => {
                    let expr = self.parse_inline_expr(&expr_str, span)?;
                    result.push(FmtStringPart::Expr(Box::new(expr)));
                }
            }
        }

        Ok(Expr::new(ExprKind::FmtString(result), span))
    }

    fn parse_inline_expr(&self, code: &str, span: aelys_syntax::Span) -> Result<Expr> {
        let source = Source::new("<fmt-expr>", code);
        let lexer = Lexer::with_source(Arc::clone(&source));
        let tokens = lexer.scan().map_err(|e| {
            CompileError::new(
                CompileErrorKind::UnexpectedToken {
                    expected: "expression".to_string(),
                    found: format!("invalid expression in format string: {}", e),
                },
                span,
                Arc::clone(&self.source),
            )
        })?;

        let mut parser = Parser::new(tokens, source);
        parser.expression().map_err(|e| {
            CompileError::new(
                CompileErrorKind::UnexpectedToken {
                    expected: "expression".to_string(),
                    found: format!("invalid expression in format string: {}", e),
                },
                span,
                Arc::clone(&self.source),
            )
            .into()
        })
    }
}

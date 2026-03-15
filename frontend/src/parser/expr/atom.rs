use super::Parser;
use crate::lexer::Lexer;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::{
    Expr, ExprKind, FmtPart, FmtStringPart, MatchArm, Pattern, Source, Stmt, StmtKind,
    StructFieldInit, TokenKind,
};
use std::sync::Arc;

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

    pub(super) fn match_expression(&mut self, start_span: aelys_syntax::Span) -> Result<Expr> {
        let scrutinee = self.expression()?;
        self.consume(&TokenKind::LBrace, "{")?;

        let mut arms = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            // Skip any stray semicolons between arms
            if self.match_token(&TokenKind::Semicolon) {
                continue;
            }
            let arm_start = self.peek().span;
            let pattern = self.parse_pattern()?;
            self.consume(&TokenKind::FatArrow, "=>")?;
            let body = if self.check(&TokenKind::Return) {
                self.match_arm_return()?
            } else {
                self.expression()?
            };
            let arm_end = self.previous().span;
            arms.push(MatchArm {
                pattern,
                body: Box::new(body),
                span: arm_start.merge(arm_end),
            });
            // Arms are separated by commas or semicolons (auto-inserted)
            if !self.match_token(&TokenKind::Comma) {
                // Allow semicolons as arm separators too (from auto-semicolon insertion)
                self.match_token(&TokenKind::Semicolon);
            }
        }
        self.consume(&TokenKind::RBrace, "}")?;
        let end_span = self.previous().span;

        Ok(Expr::new(
            ExprKind::Match {
                scrutinee: Box::new(scrutinee),
                arms,
            },
            start_span.merge(end_span),
        ))
    }

    /// Parse `return <expr>` in match arm position.
    /// Wraps the return in a Block so it fits the Expr slot the arm expects.
    fn match_arm_return(&mut self) -> Result<Expr> {
        let ret_span = self.peek().span;
        self.advance(); // consume `return`

        // Bare `return` (no value) or `return <expr>`
        let value = if self.check(&TokenKind::Comma)
            || self.check(&TokenKind::Semicolon)
            || self.check(&TokenKind::RBrace)
        {
            None
        } else {
            Some(self.expression()?)
        };

        let end_span = self.previous().span;
        let stmt = Stmt::new(
            StmtKind::Return(value),
            ret_span.merge(end_span),
        );
        Ok(Expr::new(
            ExprKind::Block {
                stmts: vec![stmt],
                tail: Box::new(Expr::new(ExprKind::Null, end_span)),
            },
            ret_span.merge(end_span),
        ))
    }

    fn parse_pattern(&mut self) -> Result<Pattern> {
        let span = self.peek().span;

        // Check for wildcard `_`
        if let TokenKind::Identifier(name) = &self.peek().kind {
            if name == "_" {
                self.advance();
                return Ok(Pattern::Wildcard(span));
            }
        }

        // Enum variant pattern: EnumName::Variant or EnumName::Variant(x, y)
        let enum_name = self.consume_identifier("enum name or _")?;
        self.consume(&TokenKind::ColonColon, "::")?;
        let variant = self.consume_identifier("variant name")?;
        let variant_end_span = self.previous().span;

        let mut bindings = Vec::new();
        if self.match_token(&TokenKind::LParen) {
            if !self.check(&TokenKind::RParen) {
                loop {
                    let binding = self.consume_identifier("binding name")?;
                    bindings.push(binding);
                    if !self.match_token(&TokenKind::Comma) {
                        break;
                    }
                }
            }
            self.consume(&TokenKind::RParen, ")")?;
        }

        let end_span = self.previous().span;

        Ok(Pattern::Variant {
            enum_name,
            variant,
            bindings,
            span: span.merge(end_span.merge(variant_end_span)),
        })
    }

    // block expr: last expr is the value (like Rust)
    pub(super) fn block_expression(&mut self) -> Result<Expr> {
        let block_start = self.previous().span;
        let mut stmts = Vec::new();

        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            // `if` without `else` is a valid statement but not a valid
            // expression (if-expression requires else).  Try as expression
            // first (handles if-else in tail position); on failure backtrack
            // and parse as statement so that guard-style `if cond { return }` works.
            if self.check(&TokenKind::If) {
                let saved = self.current;
                match self.expression() {
                    Ok(expr) => {
                        if self.check(&TokenKind::RBrace) {
                            self.consume(&TokenKind::RBrace, "}")?;
                            let end_span = self.previous().span;
                            if stmts.is_empty() {
                                return Ok(expr);
                            }
                            return Ok(Expr::new(
                                ExprKind::Block {
                                    stmts,
                                    tail: Box::new(expr),
                                },
                                block_start.merge(end_span),
                            ));
                        }
                        self.consume_semicolon()?;
                        let span = expr.span;
                        stmts.push(Stmt::new(StmtKind::Expression(expr), span));
                    }
                    Err(_) => {
                        self.current = saved;
                        stmts.push(self.declaration()?);
                    }
                }
                continue;
            }

            if self.is_expression_start() {
                let expr = self.expression()?;

                if self.check(&TokenKind::RBrace) {
                    self.consume(&TokenKind::RBrace, "}")?;
                    let end_span = self.previous().span;

                    if stmts.is_empty() {
                        return Ok(expr);
                    }
                    return Ok(Expr::new(
                        ExprKind::Block {
                            stmts,
                            tail: Box::new(expr),
                        },
                        block_start.merge(end_span),
                    ));
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
        let end_span = self.previous().span;

        // If the last stmt is an expression statement, promote it to the tail
        // expression (Rust-style block value). This handles auto-semicolon
        // insertion: `w * h\n}` gets a `;` inserted by the lexer, but the
        // user intent is for it to be the block's return value.
        let tail = if let Some(last) = stmts.last() {
            if matches!(&last.kind, StmtKind::Expression(_)) {
                let last = stmts.pop().unwrap();
                if let StmtKind::Expression(expr) = last.kind {
                    expr
                } else {
                    unreachable!()
                }
            } else {
                Expr::new(ExprKind::Null, end_span)
            }
        } else {
            Expr::new(ExprKind::Null, end_span)
        };

        if stmts.is_empty() {
            Ok(tail)
        } else {
            Ok(Expr::new(
                ExprKind::Block {
                    stmts,
                    tail: Box::new(tail),
                },
                block_start.merge(end_span),
            ))
        }
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
                | TokenKind::LBrace
                | TokenKind::Minus
                | TokenKind::Not
                | TokenKind::If
                | TokenKind::Match
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
            TokenKind::Identifier(ref name) if name.eq_ignore_ascii_case("vec") => {
                let name = name.clone();
                return self.vec_literal(name, span);
            }
            TokenKind::Identifier(ref name)
                if name.chars().next().is_some_and(|c| c.is_uppercase())
                    && self.check(&TokenKind::LBrace)
                    && matches!(self.peek_at(1).kind, TokenKind::Identifier(_))
                    && matches!(self.peek_at(2).kind, TokenKind::Colon) =>
            {
                let name = name.clone();
                return self.struct_literal(name, span);
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

            TokenKind::Match => {
                return self.match_expression(span);
            }

            TokenKind::Fn => {
                return self.lambda_expression(span);
            }

            TokenKind::LBrace => {
                return self.block_expression();
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

    /// Parse array literal: [1, 2, 3] or sized array: [; 10] or [val; 10]
    fn array_literal(&mut self, start_span: aelys_syntax::Span) -> Result<Expr> {
        // Check for sized array syntax: [; size] (zero-initialized)
        if self.match_token(&TokenKind::Semicolon) {
            let size = self.expression()?;
            self.consume(&TokenKind::RBracket, "]")?;
            let end_span = self.previous().span;
            return Ok(Expr::new(
                ExprKind::ArraySized {
                    size: Box::new(size),
                    fill_value: None,
                },
                start_span.merge(end_span),
            ));
        }

        // empty array
        if self.check(&TokenKind::RBracket) {
            self.consume(&TokenKind::RBracket, "]")?;
            let end_span = self.previous().span;
            return Ok(Expr::new(
                ExprKind::ArrayLiteral {
                    elements: Vec::new(),
                },
                start_span.merge(end_span),
            ));
        }
        let first = self.expression()?;
        // check for [val; N] syntax
        if self.match_token(&TokenKind::Semicolon) {
            let size = self.expression()?;
            self.consume(&TokenKind::RBracket, "]")?;
            let end_span = self.previous().span;
            return Ok(Expr::new(
                ExprKind::ArraySized {
                    size: Box::new(size),
                    fill_value: Some(Box::new(first)),
                },
                start_span.merge(end_span),
            ));
        }

        // regular array literal: collect remaining elements
        let mut elements = vec![first];
        while self.match_token(&TokenKind::Comma) {
            if self.check(&TokenKind::RBracket) {
                break;
            }
            elements.push(self.expression()?);
        }

        self.consume(&TokenKind::RBracket, "]")?;
        let end_span = self.previous().span;

        Ok(Expr::new(
            ExprKind::ArrayLiteral { elements },
            start_span.merge(end_span),
        ))
    }

    /// Parse typed vec literal: Vec<Float>[1.0, 2.0]
    fn vec_literal(
        &mut self,
        _collection_name: String,
        start_span: aelys_syntax::Span,
    ) -> Result<Expr> {
        let element_type = if self.match_token(&TokenKind::Lt) {
            let type_ann = self.parse_type_annotation()?;
            self.consume(&TokenKind::Gt, ">")?;
            Some(type_ann)
        } else {
            None
        };

        self.consume(&TokenKind::LBracket, "[")?;

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
            ExprKind::VecLiteral {
                element_type,
                elements,
            },
            start_span.merge(end_span),
        ))
    }

    fn struct_literal(&mut self, name: String, start_span: aelys_syntax::Span) -> Result<Expr> {
        self.consume(&TokenKind::LBrace, "{")?;

        let mut fields = Vec::new();
        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            let field_span = self.peek().span;
            let field_name = self.consume_identifier("field name")?;
            self.consume(&TokenKind::Colon, ":")?;
            let value = self.expression()?;
            let end_span = self.previous().span;

            fields.push(StructFieldInit {
                name: field_name,
                value: Box::new(value),
                span: field_span.merge(end_span),
            });

            if !self.match_token(&TokenKind::Comma) {
                break;
            }
        }

        self.consume(&TokenKind::RBrace, "}")?;
        let end_span = self.previous().span;

        Ok(Expr::new(
            ExprKind::StructLiteral { name, fields },
            start_span.merge(end_span),
        ))
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
        let mut expr = parser.expression().map_err(|e| {
            aelys_common::error::AelysError::Compile(CompileError::new(
                CompileErrorKind::UnexpectedToken {
                    expected: "expression".to_string(),
                    found: format!("invalid expression in format string: {}", e),
                },
                span,
                Arc::clone(&self.source),
            ))
        })?;

        remap_expr_spans(&mut expr, span);
        Ok(expr)
    }
}

// recursively remap all spans in an expression tree to a single span.
// used to fix format string interpolation spans so errors point to the
// string literal rather than a synthetic `<fmt-expr>` source
fn remap_expr_spans(expr: &mut Expr, span: aelys_syntax::Span) {
    expr.span = span;
    match &mut expr.kind {
        ExprKind::Binary { left, right, .. } => {
            remap_expr_spans(left, span);
            remap_expr_spans(right, span);
        }
        ExprKind::Unary { operand, .. } => {
            remap_expr_spans(operand, span);
        }
        ExprKind::And { left, right } | ExprKind::Or { left, right } => {
            remap_expr_spans(left, span);
            remap_expr_spans(right, span);
        }
        ExprKind::Call { callee, args } => {
            remap_expr_spans(callee, span);
            for arg in args {
                remap_expr_spans(arg, span);
            }
        }
        ExprKind::Assign { value, .. } => {
            remap_expr_spans(value, span);
        }
        ExprKind::Grouping(inner) => {
            remap_expr_spans(inner, span);
        }
        ExprKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_spans(condition, span);
            remap_expr_spans(then_branch, span);
            remap_expr_spans(else_branch, span);
        }
        ExprKind::Lambda { body, .. } => {
            for stmt in body {
                remap_stmt_spans(stmt, span);
            }
        }
        ExprKind::Member { object, .. } => {
            remap_expr_spans(object, span);
        }
        ExprKind::ArrayLiteral { elements, .. } | ExprKind::VecLiteral { elements, .. } => {
            for el in elements {
                remap_expr_spans(el, span);
            }
        }
        ExprKind::ArraySized {
            size, fill_value, ..
        } => {
            remap_expr_spans(size, span);
            if let Some(fv) = fill_value {
                remap_expr_spans(fv, span);
            }
        }
        ExprKind::Index { object, index } => {
            remap_expr_spans(object, span);
            remap_expr_spans(index, span);
        }
        ExprKind::IndexAssign {
            object,
            index,
            value,
        } => {
            remap_expr_spans(object, span);
            remap_expr_spans(index, span);
            remap_expr_spans(value, span);
        }
        ExprKind::FieldAssign {
            object,
            field: _,
            value,
        } => {
            remap_expr_spans(object, span);
            remap_expr_spans(value, span);
        }
        ExprKind::Range { start, end, .. } => {
            if let Some(s) = start {
                remap_expr_spans(s, span);
            }
            if let Some(e) = end {
                remap_expr_spans(e, span);
            }
        }
        ExprKind::Slice { object, range } => {
            remap_expr_spans(object, span);
            remap_expr_spans(range, span);
        }
        ExprKind::FmtString(parts) => {
            for part in parts {
                if let FmtStringPart::Expr(e) = part {
                    remap_expr_spans(e, span);
                }
            }
        }
        ExprKind::StructLiteral { fields, .. } => {
            for field in fields {
                remap_expr_spans(&mut field.value, span);
            }
        }
        ExprKind::Cast { expr, .. } => {
            remap_expr_spans(expr, span);
        }
        ExprKind::Match { scrutinee, arms } => {
            remap_expr_spans(scrutinee, span);
            for arm in arms {
                arm.span = span;
                remap_expr_spans(&mut arm.body, span);
            }
        }
        // Leaf nodes: Int, Float, String, Bool, Null, Identifier
        _ => {}
    }
}

fn remap_stmt_spans(stmt: &mut Stmt, span: aelys_syntax::Span) {
    stmt.span = span;
    match &mut stmt.kind {
        StmtKind::Expression(expr) => remap_expr_spans(expr, span),
        StmtKind::Let { initializer, .. } => remap_expr_spans(initializer, span),
        StmtKind::Block(stmts) => {
            for s in stmts {
                remap_stmt_spans(s, span);
            }
        }
        StmtKind::If {
            condition,
            then_branch,
            else_branch,
        } => {
            remap_expr_spans(condition, span);
            remap_stmt_spans(then_branch, span);
            if let Some(e) = else_branch {
                remap_stmt_spans(e, span);
            }
        }
        StmtKind::While { condition, body } => {
            remap_expr_spans(condition, span);
            remap_stmt_spans(body, span);
        }
        StmtKind::For {
            start,
            end,
            step,
            body,
            ..
        } => {
            remap_expr_spans(start, span);
            remap_expr_spans(end, span);
            if let Some(s) = step.as_mut() {
                remap_expr_spans(s, span);
            }
            remap_stmt_spans(body, span);
        }
        StmtKind::Return(Some(expr)) => remap_expr_spans(expr, span),
        StmtKind::Function(func) => {
            for s in &mut func.body {
                remap_stmt_spans(s, span);
            }
        }
        _ => {}
    }
}

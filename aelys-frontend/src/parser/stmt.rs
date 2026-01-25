use super::Parser;
use aelys_common::Result;
use aelys_common::error::CompileErrorKind;
use aelys_syntax::{Stmt, StmtKind, TokenKind};

impl Parser {
    pub fn statement(&mut self) -> Result<Stmt> {
        if self.match_token(&TokenKind::If) {
            return self.if_statement();
        }

        if self.match_token(&TokenKind::While) {
            return self.while_statement();
        }

        if self.match_token(&TokenKind::For) {
            return self.for_statement();
        }

        if self.match_token(&TokenKind::Break) {
            let span = self.previous().span;
            self.consume_semicolon()?;
            return Ok(Stmt::new(StmtKind::Break, span));
        }

        if self.match_token(&TokenKind::Continue) {
            let span = self.previous().span;
            self.consume_semicolon()?;
            return Ok(Stmt::new(StmtKind::Continue, span));
        }

        if self.match_token(&TokenKind::Return) {
            return self.return_statement();
        }

        if self.match_token(&TokenKind::LBrace) {
            return Ok(Stmt::new(
                StmtKind::Block(self.block_statements()?),
                self.previous().span,
            ));
        }

        self.expression_statement()
    }

    fn if_statement(&mut self) -> Result<Stmt> {
        let start_span = self.previous().span;
        let condition = self.expression()?;
        self.consume(&TokenKind::LBrace, "{")?;
        let then_branch = self.block_statement()?;

        let else_branch = if self.match_token(&TokenKind::Else) {
            if self.match_token(&TokenKind::If) {
                Some(Box::new(self.if_statement()?))
            } else {
                self.consume(&TokenKind::LBrace, "{")?;
                Some(Box::new(self.block_statement()?))
            }
        } else {
            None
        };

        let end_span = self.previous().span;

        Ok(Stmt::new(
            StmtKind::If {
                condition,
                then_branch: Box::new(then_branch),
                else_branch,
            },
            start_span.merge(end_span),
        ))
    }

    fn while_statement(&mut self) -> Result<Stmt> {
        let start_span = self.previous().span;
        let condition = self.expression()?;
        self.consume(&TokenKind::LBrace, "{")?;
        let body = self.block_statement()?;
        let end_span = self.previous().span;

        Ok(Stmt::new(
            StmtKind::While {
                condition,
                body: Box::new(body),
            },
            start_span.merge(end_span),
        ))
    }

    fn for_statement(&mut self) -> Result<Stmt> {
        let start_span = self.previous().span;
        let iterator = self.consume_identifier("loop variable")?;
        self.consume(&TokenKind::In, "in")?;

        let start = self.expression()?;
        let inclusive = if self.match_token(&TokenKind::DotDotEq) {
            true
        } else {
            self.consume(&TokenKind::DotDot, "..")?;
            false
        };
        let end = self.expression()?;

        let step = if self.match_token(&TokenKind::Step) {
            Some(self.expression()?)
        } else {
            None
        };

        self.consume(&TokenKind::LBrace, "{")?;
        let body = self.block_statement()?;
        let end_span = self.previous().span;

        Ok(Stmt::new(
            StmtKind::For {
                iterator,
                start,
                end,
                inclusive,
                step,
                body: Box::new(body),
            },
            start_span.merge(end_span),
        ))
    }

    fn return_statement(&mut self) -> Result<Stmt> {
        let start_span = self.previous().span;

        if self.check(&TokenKind::Semicolon) {
            self.consume_semicolon()?;
            return Ok(Stmt::new(StmtKind::Return(None), start_span));
        }

        let value = self.expression()?;
        self.consume_semicolon()?;
        let end_span = self.previous().span;

        Ok(Stmt::new(
            StmtKind::Return(Some(value)),
            start_span.merge(end_span),
        ))
    }

    fn expression_statement(&mut self) -> Result<Stmt> {
        let expr = self.expression()?;
        self.consume_semicolon()?;
        let span = expr.span;
        Ok(Stmt::new(StmtKind::Expression(expr), span))
    }

    fn block_statement(&mut self) -> Result<Stmt> {
        let start_span = self.previous().span;
        let stmts = self.block_statements()?;
        let end_span = self.previous().span;
        Ok(Stmt::new(
            StmtKind::Block(stmts),
            start_span.merge(end_span),
        ))
    }

    pub(crate) fn block_statements(&mut self) -> Result<Vec<Stmt>> {
        let mut stmts = Vec::new();

        while !self.check(&TokenKind::RBrace) && !self.is_at_end() {
            if self.match_token(&TokenKind::Semicolon) {
                continue;
            }
            stmts.push(self.declaration()?);
        }

        self.consume(&TokenKind::RBrace, "}")?;
        Ok(stmts)
    }

    pub(crate) fn consume_semicolon(&mut self) -> Result<()> {
        if self.match_token(&TokenKind::Semicolon) {
            Ok(())
        } else if self.check(&TokenKind::RBrace) {
            Ok(())
        } else {
            Err(self.error(CompileErrorKind::UnexpectedToken {
                expected: "semicolon or newline".to_string(),
                found: self.peek().kind.to_string(),
            }))
        }
    }
}

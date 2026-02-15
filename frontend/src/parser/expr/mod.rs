// pratt-style precedence climbing

mod atom;
mod binary;
mod call;
mod unary;

use super::Parser;
use aelys_common::Result;
use aelys_common::error::{CompileError, CompileErrorKind};
use aelys_syntax::{BinaryOp, Expr, ExprKind, TokenKind};
use std::sync::Arc;

impl Parser {
    pub fn expression(&mut self) -> Result<Expr> {
        self.enter_recursion()?;
        let result = self.assignment();
        self.exit_recursion();
        result
    }

    fn assignment(&mut self) -> Result<Expr> {
        let expr = self.or_expr()?;

        if self.match_token(&TokenKind::Eq) {
            let value = self.assignment()?;

            if let ExprKind::Identifier(name) = expr.kind {
                let span = expr.span.merge(value.span);
                return Ok(Expr::new(
                    ExprKind::Assign {
                        name,
                        value: Box::new(value),
                    },
                    span,
                ));
            }

            // Index assignment: arr[i] = value
            if let ExprKind::Index { object, index } = expr.kind {
                let span = object.span.merge(value.span);
                return Ok(Expr::new(
                    ExprKind::IndexAssign {
                        object,
                        index,
                        value: Box::new(value),
                    },
                    span,
                ));
            }

            return Err(CompileError::new(
                CompileErrorKind::InvalidAssignmentTarget,
                expr.span,
                Arc::clone(&self.source),
            )
            .into());
        }

        // compound assignment: x += y → x = x + y
        if let Some(op) = self.match_compound_assign() {
            let rhs = self.assignment()?;

            if let ExprKind::Identifier(ref name) = expr.kind {
                let binary = Expr::new(
                    ExprKind::Binary {
                        left: Box::new(expr.clone()),
                        op,
                        right: Box::new(rhs),
                    },
                    expr.span.merge(self.previous().span),
                );
                let span = expr.span.merge(binary.span);
                return Ok(Expr::new(
                    ExprKind::Assign {
                        name: name.clone(),
                        value: Box::new(binary),
                    },
                    span,
                ));
            }

            // index compound assignment: arr[i] += y → arr[i] = arr[i] + y
            if let ExprKind::Index {
                ref object,
                ref index,
            } = expr.kind
            {
                let binary = Expr::new(
                    ExprKind::Binary {
                        left: Box::new(expr.clone()),
                        op,
                        right: Box::new(rhs),
                    },
                    expr.span.merge(self.previous().span),
                );
                let span = object.span.merge(binary.span);
                return Ok(Expr::new(
                    ExprKind::IndexAssign {
                        object: object.clone(),
                        index: index.clone(),
                        value: Box::new(binary),
                    },
                    span,
                ));
            }

            return Err(CompileError::new(
                CompileErrorKind::InvalidAssignmentTarget,
                expr.span,
                Arc::clone(&self.source),
            )
            .into());
        }

        Ok(expr)
    }

    fn match_compound_assign(&mut self) -> Option<BinaryOp> {
        let op = match self.peek().kind {
            TokenKind::PlusEq => BinaryOp::Add,
            TokenKind::MinusEq => BinaryOp::Sub,
            TokenKind::StarEq => BinaryOp::Mul,
            TokenKind::SlashEq => BinaryOp::Div,
            TokenKind::PercentEq => BinaryOp::Mod,
            _ => return None,
        };
        self.advance();
        Some(op)
    }

    fn or_expr(&mut self) -> Result<Expr> {
        let mut left = self.and_expr()?;

        while self.match_token(&TokenKind::Or) {
            let right = self.and_expr()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Or {
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            );
        }

        Ok(left)
    }

    fn and_expr(&mut self) -> Result<Expr> {
        let mut left = self.bit_or()?;

        while self.match_token(&TokenKind::And) {
            let right = self.bit_or()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::And {
                    left: Box::new(left),
                    right: Box::new(right),
                },
                span,
            );
        }

        Ok(left)
    }

    pub fn match_binary_op(&mut self, kinds: &[TokenKind]) -> Option<BinaryOp> {
        for kind in kinds {
            if self.check(kind) {
                self.advance();
                return token_to_binary_op(kind);
            }
        }
        None
    }
}

fn token_to_binary_op(kind: &TokenKind) -> Option<BinaryOp> {
    match kind {
        TokenKind::Plus => Some(BinaryOp::Add),
        TokenKind::Minus => Some(BinaryOp::Sub),
        TokenKind::Star => Some(BinaryOp::Mul),
        TokenKind::Slash => Some(BinaryOp::Div),
        TokenKind::Percent => Some(BinaryOp::Mod),
        TokenKind::EqEq => Some(BinaryOp::Eq),
        TokenKind::BangEq => Some(BinaryOp::Ne),
        TokenKind::Lt => Some(BinaryOp::Lt),
        TokenKind::LtEq => Some(BinaryOp::Le),
        TokenKind::Gt => Some(BinaryOp::Gt),
        TokenKind::GtEq => Some(BinaryOp::Ge),
        // Bitwise operators
        TokenKind::Shl => Some(BinaryOp::Shl),
        TokenKind::Shr => Some(BinaryOp::Shr),
        TokenKind::Ampersand => Some(BinaryOp::BitAnd),
        TokenKind::Pipe => Some(BinaryOp::BitOr),
        TokenKind::Caret => Some(BinaryOp::BitXor),
        _ => None,
    }
}

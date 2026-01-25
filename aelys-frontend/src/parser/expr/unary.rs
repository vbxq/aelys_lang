use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Expr, ExprKind, TokenKind, UnaryOp};

impl Parser {
    pub(super) fn unary(&mut self) -> Result<Expr> {
        if self.match_token(&TokenKind::Minus) {
            let start = self.previous().span;
            let operand = self.unary()?;
            let span = start.merge(operand.span);
            return Ok(Expr::new(
                ExprKind::Unary {
                    op: UnaryOp::Neg,
                    operand: Box::new(operand),
                },
                span,
            ));
        }

        if self.match_token(&TokenKind::Not) {
            let start = self.previous().span;
            let operand = self.unary()?;
            let span = start.merge(operand.span);
            return Ok(Expr::new(
                ExprKind::Unary {
                    op: UnaryOp::Not,
                    operand: Box::new(operand),
                },
                span,
            ));
        }

        if self.match_token(&TokenKind::Tilde) {
            let start = self.previous().span;
            let operand = self.unary()?;
            let span = start.merge(operand.span);
            return Ok(Expr::new(
                ExprKind::Unary {
                    op: UnaryOp::BitNot,
                    operand: Box::new(operand),
                },
                span,
            ));
        }

        self.call()
    }
}

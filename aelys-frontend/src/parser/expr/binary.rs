// binary ops by precedence: or > xor > and > eq > cmp > shift > term > factor

use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Expr, ExprKind, TokenKind};

impl Parser {
    pub(super) fn bit_or(&mut self) -> Result<Expr> {
        let mut left = self.bit_xor()?;

        while let Some(op) = self.match_binary_op(&[TokenKind::Pipe]) {
            let right = self.bit_xor()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            );
        }

        Ok(left)
    }

    fn bit_xor(&mut self) -> Result<Expr> {
        let mut left = self.bit_and()?;

        while let Some(op) = self.match_binary_op(&[TokenKind::Caret]) {
            let right = self.bit_and()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            );
        }

        Ok(left)
    }

    fn bit_and(&mut self) -> Result<Expr> {
        let mut left = self.equality()?;

        while let Some(op) = self.match_binary_op(&[TokenKind::Ampersand]) {
            let right = self.equality()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            );
        }

        Ok(left)
    }

    fn equality(&mut self) -> Result<Expr> {
        let mut left = self.comparison()?;

        while let Some(op) = self.match_binary_op(&[TokenKind::EqEq, TokenKind::BangEq]) {
            let right = self.comparison()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            );
        }

        Ok(left)
    }

    fn comparison(&mut self) -> Result<Expr> {
        let mut left = self.shift()?;

        while let Some(op) = self.match_binary_op(&[
            TokenKind::Lt,
            TokenKind::LtEq,
            TokenKind::Gt,
            TokenKind::GtEq,
        ]) {
            let right = self.shift()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            );
        }

        Ok(left)
    }

    fn shift(&mut self) -> Result<Expr> {
        let mut left = self.term()?;

        while let Some(op) = self.match_binary_op(&[TokenKind::Shl, TokenKind::Shr]) {
            let right = self.term()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            );
        }

        Ok(left)
    }

    fn term(&mut self) -> Result<Expr> {
        let mut left = self.factor()?;

        while let Some(op) = self.match_binary_op(&[TokenKind::Plus, TokenKind::Minus]) {
            let right = self.factor()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            );
        }

        Ok(left)
    }

    fn factor(&mut self) -> Result<Expr> {
        let mut left = self.unary()?;

        while let Some(op) =
            self.match_binary_op(&[TokenKind::Star, TokenKind::Slash, TokenKind::Percent])
        {
            let right = self.unary()?;
            let span = left.span.merge(right.span);
            left = Expr::new(
                ExprKind::Binary {
                    left: Box::new(left),
                    op,
                    right: Box::new(right),
                },
                span,
            );
        }

        Ok(left)
    }
}

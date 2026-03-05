// binary ops by precedence: or > xor > and > eq > cmp > shift > term > factor

use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Expr, ExprKind, TokenKind};

impl Parser {
    pub(super) fn bit_or(&mut self) -> Result<Expr> {
        self.parse_left_assoc(Self::bit_xor, &[TokenKind::Pipe])
    }

    fn bit_xor(&mut self) -> Result<Expr> {
        self.parse_left_assoc(Self::bit_and, &[TokenKind::Caret])
    }

    fn bit_and(&mut self) -> Result<Expr> {
        self.parse_left_assoc(Self::equality, &[TokenKind::Ampersand])
    }

    fn equality(&mut self) -> Result<Expr> {
        self.parse_left_assoc(Self::comparison, &[TokenKind::EqEq, TokenKind::BangEq])
    }

    fn comparison(&mut self) -> Result<Expr> {
        self.parse_left_assoc(
            Self::shift,
            &[
                TokenKind::Lt,
                TokenKind::LtEq,
                TokenKind::Gt,
                TokenKind::GtEq,
            ],
        )
    }

    fn shift(&mut self) -> Result<Expr> {
        self.parse_left_assoc(Self::term, &[TokenKind::Shl, TokenKind::Shr])
    }

    fn term(&mut self) -> Result<Expr> {
        self.parse_left_assoc(Self::factor, &[TokenKind::Plus, TokenKind::Minus])
    }

    fn factor(&mut self) -> Result<Expr> {
        self.parse_left_assoc(
            Self::unary,
            &[TokenKind::Star, TokenKind::Slash, TokenKind::Percent],
        )
    }

    fn parse_left_assoc(
        &mut self,
        lower_precedence: fn(&mut Self) -> Result<Expr>,
        operators: &[TokenKind],
    ) -> Result<Expr> {
        let mut left = lower_precedence(self)?;

        while let Some(op) = self.match_binary_op(operators) {
            let right = lower_precedence(self)?;
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

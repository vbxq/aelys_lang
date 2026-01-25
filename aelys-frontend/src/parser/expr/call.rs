use super::Parser;
use aelys_common::Result;
use aelys_syntax::{Expr, ExprKind, TokenKind};

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
            } else {
                break;
            }
        }

        Ok(expr)
    }
}

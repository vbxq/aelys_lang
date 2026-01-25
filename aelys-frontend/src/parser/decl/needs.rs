// needs stmt: module imports (Python-style syntax)

use super::Parser;
use aelys_common::Result;
use aelys_syntax::{ImportKind, NeedsStmt, Stmt, StmtKind, TokenKind};

impl Parser {
    pub(super) fn needs_declaration(&mut self) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance();

        let first_ident = self.consume_identifier("module name or symbol")?;

        if self.match_token(&TokenKind::Comma) {
            let mut symbols = vec![first_ident];
            symbols.push(self.consume_identifier("symbol name")?);

            while self.match_token(&TokenKind::Comma) {
                symbols.push(self.consume_identifier("symbol name")?);
            }

            self.consume(&TokenKind::From, "from")?;
            let mut path = vec![self.consume_identifier("module name")?];

            while self.match_token(&TokenKind::Dot) {
                path.push(self.consume_identifier("module path segment")?);
            }

            self.consume_semicolon()?;
            let end_span = self.previous().span;

            return Ok(Stmt::new(
                StmtKind::Needs(NeedsStmt {
                    path,
                    kind: ImportKind::Symbols(symbols),
                    span: start_span.merge(end_span),
                }),
                start_span.merge(end_span),
            ));
        }

        if self.match_token(&TokenKind::From) {
            let symbols = vec![first_ident];
            let mut path = vec![self.consume_identifier("module name")?];

            while self.match_token(&TokenKind::Dot) {
                path.push(self.consume_identifier("module path segment")?);
            }

            self.consume_semicolon()?;
            let end_span = self.previous().span;

            return Ok(Stmt::new(
                StmtKind::Needs(NeedsStmt {
                    path,
                    kind: ImportKind::Symbols(symbols),
                    span: start_span.merge(end_span),
                }),
                start_span.merge(end_span),
            ));
        }

        let mut path = vec![first_ident];

        while self.match_token(&TokenKind::Dot) {
            if self.match_token(&TokenKind::Star) {
                self.consume_semicolon()?;
                let end_span = self.previous().span;
                return Ok(Stmt::new(
                    StmtKind::Needs(NeedsStmt {
                        path,
                        kind: ImportKind::Wildcard,
                        span: start_span.merge(end_span),
                    }),
                    start_span.merge(end_span),
                ));
            }

            path.push(self.consume_identifier("module path segment")?);
        }

        let kind = if self.match_token(&TokenKind::As) {
            let alias = self.consume_identifier("alias name")?;
            ImportKind::Module { alias: Some(alias) }
        } else {
            ImportKind::Module { alias: None }
        };

        self.consume_semicolon()?;
        let end_span = self.previous().span;

        Ok(Stmt::new(
            StmtKind::Needs(NeedsStmt {
                path,
                kind,
                span: start_span.merge(end_span),
            }),
            start_span.merge(end_span),
        ))
    }
}

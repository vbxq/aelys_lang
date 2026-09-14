
use super::Parser;
use aelys_common::Result;
use aelys_syntax::{ImportKind, NeedsStmt, NeedsTarget, Stmt, StmtKind, TokenKind};

impl Parser {
    pub(crate) fn needs_declaration(&mut self) -> Result<Stmt> {
        let start_span = self.peek().span;
        self.advance();

        // one token of lookahead classifies the target, and nothing downstream re-decides it
        if let TokenKind::String(header) = &self.peek().kind {
            let header = header.clone();
            self.advance();
            self.consume_semicolon()?;
            let end_span = self.previous().span;
            let span = start_span.merge(end_span);
            return Ok(Stmt::new(
                StmtKind::Needs(NeedsStmt {
                    target: NeedsTarget::Foreign { header },
                    span,
                }),
                span,
            ));
        }

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

            return Ok(module_needs(
                path,
                ImportKind::Symbols(symbols),
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

            return Ok(module_needs(
                path,
                ImportKind::Symbols(symbols),
                start_span.merge(end_span),
            ));
        }

        let mut path = vec![first_ident];

        while self.match_token(&TokenKind::Dot) {
            if self.match_token(&TokenKind::Star) {
                self.consume_semicolon()?;
                let end_span = self.previous().span;
                return Ok(module_needs(
                    path,
                    ImportKind::Wildcard,
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

        Ok(module_needs(path, kind, start_span.merge(end_span)))
    }
}

fn module_needs(path: Vec<String>, kind: ImportKind, span: aelys_syntax::Span) -> Stmt {
    Stmt::new(
        StmtKind::Needs(NeedsStmt {
            target: NeedsTarget::Module { path, kind },
            span,
        }),
        span,
    )
}

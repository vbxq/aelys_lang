mod block;
mod expr;
mod items;
mod macros;
mod pattern;
mod types;

use aelys_common::Diagnostic;
use aelys_common::Result;
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_syntax::{Source, Span, Stmt, Token, TokenKind};
use std::sync::Arc;

const MAX_RECURSION_DEPTH: usize = 1000;

pub struct Parser {
    tokens: Vec<Token>,
    current: usize,
    source: Arc<Source>,
    recursion_depth: usize,
    block_depth: usize,
    errors: Vec<Diagnostic>,
    pattern_in_let: bool,
    no_struct: bool,
    mut_self: bool,
    loops: Vec<block::LoopCtx>,
    in_closure: bool,
    temps: usize,
    fn_returns: Vec<bool>,
    fmt_writes: Option<Vec<(String, Span)>>,
}

impl Parser {
    pub fn new(tokens: Vec<Token>, source: Arc<Source>) -> Self {
        Self {
            tokens,
            current: 0,
            source,
            recursion_depth: 0,
            block_depth: 0,
            errors: Vec::new(),
            pattern_in_let: false,
            no_struct: false,
            mut_self: false,
            loops: Vec::new(),
            in_closure: false,
            temps: 0,
            fn_returns: Vec::new(),
            fmt_writes: None,
        }
    }

    pub fn parse(mut self) -> Result<Vec<Stmt>> {
        let mut statements = Vec::new();
        while !self.is_at_end() {
            let start = self.current;
            if self.check(&TokenKind::Semicolon) {
                let span = self.peek().span;
                self.advance();
                let diag = self.rule(111, "unexpected `;` after an item; remove it", span);
                self.errors.push(diag.to_diagnostic());
                continue;
            }
            match self.item() {
                Ok(stmts) => statements.extend(stmts),
                Err(err) => {
                    match err {
                        AelysError::Compile(e) => self.errors.push(e.to_diagnostic()),
                        AelysError::Multiple(diags) => self.errors.extend(diags.into_iter().take(1)),
                    }
                    self.synchronize(start);
                }
            }
        }
        if self.errors.is_empty() {
            Ok(statements)
        } else {
            Err(AelysError::Multiple(self.errors))
        }
    }

    fn synchronize(&mut self, start: usize) {
        if self.current == start && !self.is_at_end() {
            self.advance();
        }
        let mut depth = 0usize;
        for token in &self.tokens[start..self.current] {
            match token.kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => depth = depth.saturating_sub(1),
                _ => {}
            }
        }
        while !self.is_at_end() {
            match &self.peek().kind {
                TokenKind::LBrace => depth += 1,
                TokenKind::RBrace => depth = depth.saturating_sub(1),
                _ if depth == 0 && self.starts_item() => return,
                _ => {}
            }
            self.advance();
        }
    }

    fn error(&self, kind: CompileErrorKind) -> AelysError {
        CompileError::new(kind, self.peek().span, Arc::clone(&self.source)).into()
    }

    fn rule(&self, code: u16, message: &str, span: Span) -> CompileError {
        self.rule_help(code, message, None, span)
    }

    fn rule_help(&self, code: u16, message: &str, help: Option<&str>, span: Span) -> CompileError {
        CompileError::new(
            CompileErrorKind::ParseRule {
                code,
                message: message.to_string(),
                help: help.map(str::to_string),
            },
            span,
            Arc::clone(&self.source),
        )
    }

    fn fail<T>(&self, code: u16, message: &str, span: Span) -> Result<T> {
        Err(self.rule(code, message, span).into())
    }

    fn fail_help<T>(&self, code: u16, message: &str, help: &str, span: Span) -> Result<T> {
        Err(self.rule_help(code, message, Some(help), span).into())
    }

    fn refuse_in_place(&mut self, kind: CompileErrorKind, span: Span) {
        let diag = CompileError::new(kind, span, Arc::clone(&self.source)).to_diagnostic();
        self.errors.push(diag);
    }

    fn unexpected<T>(&self, expected: &str) -> Result<T> {
        Err(self.error(CompileErrorKind::UnexpectedToken {
            expected: expected.to_string(),
            found: self.peek().kind.to_string(),
        }))
    }

    fn enter_recursion(&mut self) -> Result<()> {
        self.recursion_depth += 1;
        if self.recursion_depth > MAX_RECURSION_DEPTH {
            return Err(self.error(CompileErrorKind::RecursionDepthExceeded {
                max: MAX_RECURSION_DEPTH,
            }));
        }
        Ok(())
    }

    fn exit_recursion(&mut self) {
        self.recursion_depth = self.recursion_depth.saturating_sub(1);
    }

    fn consume(&mut self, kind: &TokenKind, expected: &str) -> Result<()> {
        if self.check(kind) {
            self.advance();
            Ok(())
        } else {
            self.unexpected(expected)
        }
    }

    fn after_previous(&self) -> Span {
        let end = self.previous().span.end;
        let (line, column) = self.source.line_col_at_offset(end);
        Span::new(end, end, line, column)
    }

    fn expect_semi(&mut self, what: &str) -> Result<()> {
        if self.match_token(&TokenKind::Semicolon) {
            return Ok(());
        }
        let found = self.peek().kind.to_string();
        Err(self
            .rule_help(
                110,
                &format!("expected `;` after {what}, found {found}"),
                Some("add `;` here"),
                self.after_previous(),
            )
            .into())
    }

    fn consume_gt(&mut self) -> Result<()> {
        let split = |this: &mut Self, first: TokenKind, rest: TokenKind| {
            let span = this.tokens[this.current].span;
            let head = Span::new(span.start, span.start + 1, span.line, span.column);
            let tail = Span::new(span.start + 1, span.end, span.line, span.column + 1);
            this.tokens[this.current] = Token::new(rest, tail);
            this.tokens.insert(this.current, Token::new(first, head));
            this.advance();
        };
        match self.peek().kind {
            TokenKind::Gt => {
                self.advance();
                Ok(())
            }
            TokenKind::Shr => {
                split(self, TokenKind::Gt, TokenKind::Gt);
                Ok(())
            }
            TokenKind::GtEq => {
                split(self, TokenKind::Gt, TokenKind::Eq);
                Ok(())
            }
            TokenKind::ShrEq => {
                split(self, TokenKind::Gt, TokenKind::GtEq);
                Ok(())
            }
            _ => self.unexpected("`>`"),
        }
    }

    fn consume_identifier(&mut self, expected: &str) -> Result<String> {
        match &self.peek().kind {
            TokenKind::Identifier(name) => {
                let name = name.clone();
                self.advance();
                Ok(name)
            }
            _ => Err(self.name_error(expected)),
        }
    }

    fn name_error(&self, expected: &str) -> AelysError {
        let kind = &self.peek().kind;
        let found = match kind.keyword_spelling() {
            Some(word) => format!("reserved word `{word}`; write `r#{word}` to use it as a name"),
            None => kind.to_string(),
        };
        self.error(CompileErrorKind::UnexpectedToken {
            expected: expected.to_string(),
            found,
        })
    }

    fn consume_member_name(&mut self, expected: &str) -> Result<String> {
        let token = self.peek();
        if let Some(word) = token.kind.keyword_spelling()
            && token.span.end - token.span.start == word.len()
        {
            self.advance();
            return Ok(word.to_string());
        }
        self.consume_identifier(expected)
    }

    fn consume_value_name(&mut self, expected: &str) -> Result<String> {
        let span = self.peek().span;
        let name = self.consume_identifier(expected)?;
        self.refuse_reserved_value_name(&name, span);
        Ok(name)
    }

    fn refuse_reserved_value_name(&mut self, name: &str, span: Span) -> bool {
        let Some(enum_name) = aelys_syntax::prelude_variant_enum(name) else {
            return false;
        };
        let kind = CompileErrorKind::ReservedVariantName {
            name: name.to_string(),
            enum_name: enum_name.to_string(),
        };
        self.refuse_in_place(kind, span);
        true
    }

    fn consume_binding_name(&mut self, expected: &str) -> Result<String> {
        let span = self.peek().span;
        let name = self.consume_identifier(expected)?;
        if !self.refuse_reserved_value_name(&name, span) && name.starts_with(char::is_uppercase) {
            let kind = CompileErrorKind::UppercaseBinding { name: name.clone() };
            self.refuse_in_place(kind, span);
        }
        Ok(name)
    }

    fn check(&self, kind: &TokenKind) -> bool {
        !self.is_at_end() && self.peek().kind == *kind
    }

    fn check_ident(&self, word: &str) -> bool {
        matches!(&self.peek().kind, TokenKind::Identifier(w) if w == word)
    }

    fn match_token(&mut self, kind: &TokenKind) -> bool {
        if self.check(kind) {
            self.advance();
            true
        } else {
            false
        }
    }

    fn advance(&mut self) -> &Token {
        if !self.is_at_end() {
            self.current += 1;
        }
        self.previous()
    }

    fn is_at_end(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Eof)
    }

    fn peek(&self) -> &Token {
        &self.tokens[self.current]
    }

    fn peek_at(&self, offset: usize) -> &Token {
        let idx = self.current + offset;
        self.tokens.get(idx).unwrap_or_else(|| self.tokens.last().unwrap())
    }

    fn previous(&self) -> &Token {
        &self.tokens[self.current.saturating_sub(1)]
    }

    fn temp(&mut self, what: &str) -> String {
        self.temps += 1;
        format!("{what}#{}", self.temps)
    }

    fn adjacent(&self, a: usize, b: usize) -> bool {
        self.peek_at(a).span.end == self.peek_at(b).span.start
    }
}

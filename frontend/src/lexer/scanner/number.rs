use super::scan::{is_ident_continue, is_ident_start};
use super::{Lexer, Result};
use aelys_common::error::{AelysError, CompileErrorKind};
use aelys_syntax::{Span, Token, TokenKind};

impl Lexer {
    pub(super) fn number(&mut self) -> Result<()> {
        if matches!(self.tokens.last().map(|t| &t.kind), Some(TokenKind::Dot)) {
            return self.tuple_element();
        }
        let radix = if self.text(self.start) == "0" {
            match self.peek() {
                'x' | 'X' => 16,
                'o' | 'O' => 8,
                'b' | 'B' => 2,
                _ => 10,
            }
        } else {
            10
        };
        if radix != 10 {
            self.advance();
            return self.radix_number(radix);
        }

        while self.peek().is_ascii_digit() || self.peek() == '_' {
            self.advance();
        }
        let mut is_float = false;
        if self.peek() == '.' && self.peek_next().is_ascii_digit() {
            is_float = true;
            self.advance();
            while self.peek().is_ascii_digit() || self.peek() == '_' {
                self.advance();
            }
        }
        if matches!(self.peek(), 'e' | 'E')
            && (self.peek_next().is_ascii_digit()
                || (matches!(self.peek_next(), '+' | '-') && self.peek_at(2).is_ascii_digit()))
        {
            is_float = true;
            self.advance();
            if matches!(self.peek(), '+' | '-') {
                self.advance();
            }
            while self.peek().is_ascii_digit() || self.peek() == '_' {
                self.advance();
            }
        }
        let digits: String = self.text(self.start).chars().filter(|&c| c != '_').collect();
        let suffix = self.suffix();

        if is_float || suffix.as_deref().is_some_and(|s| s.starts_with('f')) {
            let value: f64 = digits
                .parse()
                .map_err(|_| AelysError::Compile(self.error(CompileErrorKind::InvalidNumber(digits.clone()))))?;
            match suffix {
                Some(s) => self.add_token(TokenKind::SuffixedFloat(value, s)),
                None => self.add_token(TokenKind::Float(value)),
            }
            return Ok(());
        }
        self.integer(&digits, 10, suffix)
    }

    fn radix_number(&mut self, radix: u32) -> Result<()> {
        let body = self.current;
        while self.peek().is_digit(radix) || self.peek() == '_' {
            self.advance();
        }
        let digits: String = self.text(body).chars().filter(|&c| c != '_').collect();
        let suffix = self.suffix();
        if digits.is_empty() {
            let full = self.text(self.start).to_string();
            return Err(AelysError::Compile(self.error(CompileErrorKind::InvalidNumber(full))));
        }
        self.integer(&digits, radix, suffix)
    }

    fn suffix(&mut self) -> Option<String> {
        if !is_ident_start(self.peek()) {
            return None;
        }
        let from = self.current;
        while is_ident_continue(self.peek()) {
            self.advance();
        }
        Some(self.text(from).to_string())
    }

    fn integer(&mut self, digits: &str, radix: u32, suffix: Option<String>) -> Result<()> {
        let Ok(value) = u128::from_str_radix(digits, radix) else {
            let full = self.text(self.start).to_string();
            return Err(AelysError::Compile(self.error(CompileErrorKind::InvalidNumber(full))));
        };
        if let Some(suffix) = suffix {
            self.add_token(TokenKind::SuffixedInt(value, suffix));
            return Ok(());
        }
        if let Ok(n) = i64::try_from(value) {
            self.add_token(TokenKind::Int(n));
            return Ok(());
        }
        if value == LEAST_I64_MAGNITUDE && self.negates_what_follows() {
            let minus = self.tokens.pop().expect("negates_what_follows saw it");
            self.tokens.push(Token::new(
                TokenKind::Int(i64::MIN),
                Span::new(minus.span.start, self.current, minus.span.line, minus.span.column),
            ));
            return Ok(());
        }
        match u64::try_from(value) {
            Ok(n) if !self.negates_what_follows() => {
                self.add_token(TokenKind::UInt(n));
                Ok(())
            }
            _ => {
                let full = self.text(self.start).to_string();
                Err(AelysError::Compile(self.error(CompileErrorKind::InvalidNumber(full))))
            }
        }
    }

    fn tuple_element(&mut self) -> Result<()> {
        while self.peek().is_ascii_digit() {
            self.advance();
        }
        let text = self.text(self.start).to_string();
        let plain = text == "0" || !text.starts_with('0');
        match text.parse::<i64>() {
            Ok(n) if plain && !is_ident_continue(self.peek()) => {
                self.add_token(TokenKind::Int(n));
                Ok(())
            }
            _ => Err(AelysError::Compile(self.rule(
                101,
                format!(
                    "`{text}` is not a tuple element; a tuple element is written in decimal digits, like `t.0`"
                ),
                self.span(),
            ))),
        }
    }

    fn negates_what_follows(&self) -> bool {
        let mut back = self.tokens.iter().rev();
        if !matches!(back.next().map(|t| &t.kind), Some(TokenKind::Minus)) {
            return false;
        }
        !back.next().is_some_and(|t| ends_an_expression(&t.kind))
    }
}

const LEAST_I64_MAGNITUDE: u128 = 9223372036854775808;

fn ends_an_expression(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Identifier(_)
            | TokenKind::Int(_)
            | TokenKind::UInt(_)
            | TokenKind::Float(_)
            | TokenKind::SuffixedInt(..)
            | TokenKind::SuffixedFloat(..)
            | TokenKind::String(_)
            | TokenKind::Char(_)
            | TokenKind::Byte(_)
            | TokenKind::True
            | TokenKind::False
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::RBrace
            | TokenKind::Question
    )
}

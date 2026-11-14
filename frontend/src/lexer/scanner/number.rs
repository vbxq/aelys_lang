// octal literal support contributed by keggek (ported to rust)

use super::{Lexer, Result};
use aelys_common::error::{AelysError, CompileErrorKind};
use aelys_syntax::{Span, Token, TokenKind};

impl Lexer {
    pub(super) fn number(&mut self) -> Result<()> {
        if self.chars.get(self.start) == Some(&'0') {
            if self.match_char('x') || self.match_char('X') {
                return self.hex_number();
            }
            if self.match_char('b') || self.match_char('B') {
                return self.binary_number();
            }
            if self.match_char('o') || self.match_char('O') {
                return self.octal_number();
            }
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

        if self.peek() == 'e' || self.peek() == 'E' {
            is_float = true;
            self.advance();
            if self.peek() == '+' || self.peek() == '-' {
                self.advance();
            }
            while self.peek().is_ascii_digit() || self.peek() == '_' {
                self.advance();
            }
        }

        let text: String = self.chars[self.start..self.current]
            .iter()
            .filter(|&&c| c != '_')
            .collect();

        if is_float {
            match text.parse::<f64>() {
                Ok(n) => self.add_token(TokenKind::Float(n)),
                Err(_) => {
                    return Err(AelysError::Compile(
                        self.error(CompileErrorKind::InvalidNumber(text)),
                    ));
                }
            }
        } else {
            match text.parse::<i64>() {
                Ok(n) => self.add_token(TokenKind::Int(n)),
                Err(_) if text == LEAST_I64_MAGNITUDE && self.negates_what_follows() => {
                    let minus = self.tokens.pop().expect("negates_what_follows saw it");
                    self.tokens.push(Token::new(
                        TokenKind::Int(i64::MIN),
                        Span::new(
                            minus.span.start,
                            self.current,
                            minus.span.line,
                            minus.span.column,
                        ),
                    ));
                    self.pending_semicolon = true;
                }
                Err(_) => {
                    return Err(AelysError::Compile(
                        self.error(CompileErrorKind::InvalidNumber(text)),
                    ));
                }
            }
        }

        Ok(())
    }

    // the magnitude of the least i64 has no positive value, so it is a literal only under a minus
    fn negates_what_follows(&self) -> bool {
        let mut back = self.tokens.iter().rev();
        if !matches!(back.next().map(|t| &t.kind), Some(TokenKind::Minus)) {
            return false;
        }
        !back.next().is_some_and(|t| ends_an_expression(&t.kind))
    }

    fn hex_number(&mut self) -> Result<()> {
        while self.peek().is_ascii_hexdigit() || self.peek() == '_' {
            self.advance();
        }

        let text: String = self.chars[self.start + 2..self.current]
            .iter()
            .filter(|&&c| c != '_')
            .collect();

        match i64::from_str_radix(&text, 16) {
            Ok(n) => self.add_token(TokenKind::Int(n)),
            Err(_) => {
                let full: String = self.chars[self.start..self.current].iter().collect();
                return Err(AelysError::Compile(
                    self.error(CompileErrorKind::InvalidNumber(full)),
                ));
            }
        }

        Ok(())
    }

    fn binary_number(&mut self) -> Result<()> {
        while self.peek() == '0' || self.peek() == '1' || self.peek() == '_' {
            self.advance();
        }

        let text: String = self.chars[self.start + 2..self.current]
            .iter()
            .filter(|&&c| c != '_')
            .collect();

        match i64::from_str_radix(&text, 2) {
            Ok(n) => self.add_token(TokenKind::Int(n)),
            Err(_) => {
                let full: String = self.chars[self.start..self.current].iter().collect();
                return Err(AelysError::Compile(
                    self.error(CompileErrorKind::InvalidNumber(full)),
                ));
            }
        }

        Ok(())
    }

    fn octal_number(&mut self) -> Result<()> {
        while matches!(self.peek(), '0'..='7' | '_') {
            self.advance();
        }

        let text: String = self.chars[self.start + 2..self.current]
            .iter()
            .filter(|&&c| c != '_')
            .collect();

        match i64::from_str_radix(&text, 8) {
            Ok(n) => self.add_token(TokenKind::Int(n)),
            Err(_) => {
                let full: String = self.chars[self.start..self.current].iter().collect();
                return Err(AelysError::Compile(
                    self.error(CompileErrorKind::InvalidNumber(full)),
                ));
            }
        }

        Ok(())
    }
}

const LEAST_I64_MAGNITUDE: &str = "9223372036854775808";

fn ends_an_expression(kind: &TokenKind) -> bool {
    matches!(
        kind,
        TokenKind::Identifier(_)
            | TokenKind::Int(_)
            | TokenKind::Float(_)
            | TokenKind::String(_)
            | TokenKind::FmtString(_)
            | TokenKind::True
            | TokenKind::False
            | TokenKind::Null
            | TokenKind::RParen
            | TokenKind::RBracket
            | TokenKind::RBrace
            | TokenKind::PlusPlus
            | TokenKind::MinusMinus
            | TokenKind::Question
    )
}

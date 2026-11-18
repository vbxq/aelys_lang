use super::{Lexer, Result};
use aelys_common::error::{AelysError, CompileError, CompileErrorKind};
use aelys_syntax::{Span, TokenKind};
use std::sync::Arc;

impl Lexer {
    pub(super) fn string(&mut self) -> Result<()> {
        let mut text = String::new();
        loop {
            if self.is_at_end() {
                return Err(self.unterminated_string());
            }
            match self.peek() {
                '"' => break,
                '\\' if matches!(self.peek_next(), '\n' | '\r') => {
                    self.advance();
                    while matches!(self.peek(), ' ' | '\t' | '\n' | '\r') {
                        self.advance();
                    }
                }
                '\\' => text.push(self.escape(true)?),
                _ => text.push(self.advance()),
            }
        }
        self.advance();
        self.add_token(TokenKind::String(text));
        Ok(())
    }

    pub(super) fn raw_string(&mut self) -> Result<()> {
        let mut hashes = 0;
        while self.match_char('#') {
            hashes += 1;
        }
        if !self.match_char('"') {
            return Err(AelysError::Compile(self.rule(
                101,
                "expected `\"` to open the raw string".to_string(),
                self.span(),
            )));
        }
        let body = self.current;
        loop {
            if self.is_at_end() {
                return Err(self.unterminated_string());
            }
            if self.peek() == '"' && (1..=hashes).all(|i| self.peek_at(i) == '#') {
                let text = self.text(body).to_string();
                self.advance();
                for _ in 0..hashes {
                    self.advance();
                }
                self.add_token(TokenKind::String(text));
                return Ok(());
            }
            self.advance();
        }
    }

    fn unterminated_string(&self) -> AelysError {
        AelysError::Compile(CompileError::new(
            CompileErrorKind::UnterminatedString,
            Span::new(self.start, self.start + 1, self.start_line, self.start_column),
            Arc::clone(&self.source),
        ))
    }

    pub(super) fn escape(&mut self, unicode: bool) -> Result<char> {
        let open = self.current;
        let (line, column) = (self.line, self.column);
        self.advance();
        let c = self.advance();
        let bad = |lexer: &Self, message: String| {
            AelysError::Compile(lexer.rule(
                5,
                message,
                Span::new(open, lexer.current, line, column),
            ))
        };
        Ok(match c {
            'n' => '\n',
            't' => '\t',
            'r' => '\r',
            '\\' => '\\',
            '"' => '"',
            '\'' => '\'',
            '0' => '\0',
            'x' => {
                let hex: String = [self.advance(), self.advance()].iter().collect();
                match u8::from_str_radix(&hex, 16) {
                    Ok(v) if v <= 0x7f || !unicode => v as char,
                    Ok(_) => {
                        return Err(bad(
                            self,
                            format!("`\\x{hex}` is out of range; a `\\x` escape is at most `\\x7F`, write `\\u{{{hex}}}`"),
                        ));
                    }
                    Err(_) => return Err(bad(self, format!("invalid escape `\\x{hex}`; write two hex digits"))),
                }
            }
            'u' if unicode => {
                if !self.match_char('{') {
                    return Err(bad(self, "invalid unicode escape; write `\\u{e9}`".to_string()));
                }
                let from = self.current;
                while self.peek().is_ascii_hexdigit() || self.peek() == '_' {
                    self.advance();
                }
                let digits: String = self.text(from).chars().filter(|&c| c != '_').collect();
                if !self.match_char('}') || digits.is_empty() || digits.len() > 6 {
                    return Err(bad(self, "invalid unicode escape; write one to six hex digits: `\\u{e9}`".to_string()));
                }
                match u32::from_str_radix(&digits, 16).ok().and_then(char::from_u32) {
                    Some(ch) => ch,
                    None => return Err(bad(self, format!("`\\u{{{digits}}}` is not a unicode scalar value"))),
                }
            }
            other => {
                return Err(AelysError::Compile(CompileError::new(
                    CompileErrorKind::InvalidEscape(other),
                    Span::new(open, self.current, line, column),
                    Arc::clone(&self.source),
                )));
            }
        })
    }
}

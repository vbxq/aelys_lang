use super::{Lexer, Result};
use aelys_common::error::{AelysError, CompileErrorKind};
use aelys_syntax::{FmtPart, TokenKind};

impl Lexer {
    pub(super) fn string(&mut self) -> Result<()> {
        let mut parts: Vec<FmtPart> = Vec::new();
        let mut current_literal = String::new();
        let mut has_format = false;

        while self.peek() != '"' && !self.is_at_end() {
            if self.peek() == '\n' {
                self.line += 1;
                self.column = 1;
            }

            match self.peek() {
                '\\' => {
                    self.advance();
                    let escaped = match self.peek() {
                        'n' => '\n',
                        't' => '\t',
                        'r' => '\r',
                        '\\' => '\\',
                        '"' => '"',
                        '0' => '\0',
                        '\'' => '\'',
                        c => {
                            return Err(AelysError::Compile(
                                self.error(CompileErrorKind::InvalidEscape(c)),
                            ));
                        }
                    };
                    self.advance();
                    current_literal.push(escaped);
                }
                '{' => {
                    self.advance();
                    if self.peek() == '{' {
                        // {{ -> literal {
                        self.advance();
                        current_literal.push('{');
                    } else {
                        // start of format expression or placeholder
                        has_format = true;
                        if !current_literal.is_empty() {
                            parts.push(FmtPart::Literal(std::mem::take(&mut current_literal)));
                        }

                        if self.peek() == '}' {
                            // {} -> placeholder
                            self.advance();
                            parts.push(FmtPart::Placeholder);
                        } else {
                            // {expr} -> expression
                            let expr = self.scan_format_expr()?;
                            parts.push(FmtPart::Expr(expr));
                        }
                    }
                }
                '}' => {
                    self.advance();
                    if self.peek() == '}' {
                        // }} -> literal }
                        self.advance();
                        current_literal.push('}');
                    } else {
                        return Err(AelysError::Compile(
                            self.error(CompileErrorKind::UnmatchedCloseBrace),
                        ));
                    }
                }
                _ => {
                    current_literal.push(self.advance());
                }
            }
        }

        if self.is_at_end() {
            return Err(AelysError::Compile(
                self.error(CompileErrorKind::UnterminatedString),
            ));
        }

        self.advance(); // closing "

        if has_format {
            if !current_literal.is_empty() {
                parts.push(FmtPart::Literal(current_literal));
            }
            self.add_token(TokenKind::FmtString(parts));
        } else {
            self.add_token(TokenKind::String(current_literal));
        }

        Ok(())
    }

    fn scan_format_expr(&mut self) -> Result<String> {
        let mut expr = String::new();
        let mut brace_depth = 1;

        while !self.is_at_end() {
            let c = self.peek();
            match c {
                '{' => {
                    brace_depth += 1;
                    expr.push(self.advance());
                }
                '}' => {
                    brace_depth -= 1;
                    if brace_depth == 0 {
                        self.advance();
                        return Ok(expr);
                    }
                    expr.push(self.advance());
                }
                '"' => {
                    expr.push(self.advance());
                    while !self.is_at_end() && self.peek() != '"' {
                        if self.peek() == '\\' {
                            expr.push(self.advance());
                            if !self.is_at_end() {
                                expr.push(self.advance());
                            }
                        } else {
                            if self.peek() == '\n' {
                                self.line += 1;
                                self.column = 1;
                            }
                            expr.push(self.advance());
                        }
                    }
                    if self.is_at_end() {
                        return Err(AelysError::Compile(
                            self.error(CompileErrorKind::UnterminatedFmtExpr),
                        ));
                    }
                    expr.push(self.advance());
                }
                '\n' => {
                    self.line += 1;
                    self.column = 1;
                    expr.push(self.advance());
                }
                _ => {
                    expr.push(self.advance());
                }
            }
        }

        Err(AelysError::Compile(
            self.error(CompileErrorKind::UnterminatedFmtExpr),
        ))
    }
}

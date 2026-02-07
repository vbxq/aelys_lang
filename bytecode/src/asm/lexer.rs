//! Lexer for .aasm files

use super::AssemblerError;

/// Result type for lexer operations
pub(super) type Result<T> = std::result::Result<T, AssemblerError>;

/// Token types for the assembler lexer
#[derive(Debug, Clone, PartialEq)]
pub(super) enum Token {
    /// A directive like .function, .name, .code
    Directive(String),
    /// A label reference like L0
    LabelRef(String),
    /// An identifier/opcode like LoadI, Move
    Ident(String),
    /// A register like r0, r1
    Register(u8),
    /// An integer literal
    Int(i64),
    /// A float literal
    Float(f64),
    /// A string literal (already unescaped)
    String(String),
    /// A boolean literal
    Bool(bool),
    /// Null literal
    Null,
    /// Comma separator
    Comma,
    /// Colon
    Colon,
    /// @ symbol (for absolute addresses)
    At,
    /// [ left bracket
    LBracket,
    /// ] right bracket
    RBracket,
    /// End of line
    Newline,
    /// End of file
    Eof,
}

/// Lexer for .aasm files
pub(super) struct Lexer<'a> {
    chars: std::iter::Peekable<std::str::CharIndices<'a>>,
    line: usize,
    _marker: std::marker::PhantomData<&'a str>,
}

impl<'a> Lexer<'a> {
    pub(super) fn new(source: &'a str) -> Self {
        Self {
            chars: source.char_indices().peekable(),
            line: 1,
            _marker: std::marker::PhantomData,
        }
    }

    pub(super) fn current_line(&self) -> usize {
        self.line
    }

    pub(super) fn next_token(&mut self) -> Result<Token> {
        self.skip_whitespace_and_comments();

        let Some(&(_, c)) = self.chars.peek() else {
            return Ok(Token::Eof);
        };

        match c {
            '\n' => {
                self.chars.next();
                self.line += 1;
                Ok(Token::Newline)
            }
            ',' => {
                self.chars.next();
                Ok(Token::Comma)
            }
            ':' => {
                self.chars.next();
                Ok(Token::Colon)
            }
            '@' => {
                self.chars.next();
                Ok(Token::At)
            }
            '[' => {
                self.chars.next();
                Ok(Token::LBracket)
            }
            ']' => {
                self.chars.next();
                Ok(Token::RBracket)
            }
            '.' => {
                self.chars.next();
                let name = self.read_identifier();
                Ok(Token::Directive(name))
            }
            '"' => {
                self.chars.next();
                let s = self.read_string()?;
                Ok(Token::String(s))
            }
            'r' if self.peek_is_digit(1) => {
                self.chars.next();
                let num = self.read_number()?;
                if let Token::Int(n) = num {
                    if (0..=255).contains(&n) {
                        Ok(Token::Register(n as u8))
                    } else {
                        Err(AssemblerError::InvalidRegister(format!("r{}", n)))
                    }
                } else {
                    Err(AssemblerError::InvalidRegister(
                        "register must be integer".to_string(),
                    ))
                }
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let name = self.read_identifier();
                match name.as_str() {
                    "true" => Ok(Token::Bool(true)),
                    "false" => Ok(Token::Bool(false)),
                    "null" => Ok(Token::Null),
                    _ => {
                        // Labels are L followed by digits or underscore only (L0, L1, L_else)
                        // But not opcodes like LoadI, LoadK
                        if name.starts_with('L') && name.len() > 1 {
                            let rest = &name[1..];
                            // If the rest is all digits or starts with underscore, it's a label
                            if rest
                                .chars()
                                .next()
                                .map(|c| c.is_ascii_digit() || c == '_')
                                .unwrap_or(false)
                            {
                                return Ok(Token::LabelRef(name));
                            }
                        }
                        Ok(Token::Ident(name))
                    }
                }
            }
            c if c.is_ascii_digit() || c == '-' => self.read_number(),
            _ => Err(AssemblerError::ParseError {
                line: self.line,
                message: format!("Unexpected character: '{}'", c),
            }),
        }
    }

    fn skip_whitespace_and_comments(&mut self) {
        loop {
            match self.chars.peek() {
                Some(&(_, ' ')) | Some(&(_, '\t')) | Some(&(_, '\r')) => {
                    self.chars.next();
                }
                Some(&(_, ';')) => {
                    while let Some(&(_, c)) = self.chars.peek() {
                        if c == '\n' {
                            break;
                        }
                        self.chars.next();
                    }
                }
                _ => break,
            }
        }
    }

    /// Peek ahead to see if the next character is a digit
    fn peek_is_digit(&self, _offset: usize) -> bool {
        let mut chars = self.chars.clone();
        chars.next();
        chars
            .peek()
            .map(|&(_, c)| c.is_ascii_digit())
            .unwrap_or(false)
    }

    fn read_identifier(&mut self) -> String {
        let mut name = String::new();
        while let Some(&(_, c)) = self.chars.peek() {
            if c.is_ascii_alphanumeric() || c == '_' {
                name.push(c);
                self.chars.next();
            } else {
                break;
            }
        }
        name
    }

    fn read_number(&mut self) -> Result<Token> {
        let mut num_str = String::new();
        let mut is_float = false;

        if let Some(&(_, '-')) = self.chars.peek() {
            num_str.push('-');
            self.chars.next();
        }

        while let Some(&(_, c)) = self.chars.peek() {
            if c.is_ascii_digit() {
                num_str.push(c);
                self.chars.next();
            } else if c == '.' && !is_float {
                is_float = true;
                num_str.push(c);
                self.chars.next();
            } else if (c == 'e' || c == 'E') && !num_str.contains('e') && !num_str.contains('E') {
                is_float = true;
                num_str.push(c);
                self.chars.next();
                if let Some(&(_, sign)) = self.chars.peek()
                    && (sign == '+' || sign == '-')
                {
                    num_str.push(sign);
                    self.chars.next();
                }
            } else {
                break;
            }
        }

        if num_str == "-" {
            let rest = self.read_identifier();
            if rest == "inf" {
                return Ok(Token::Float(f64::NEG_INFINITY));
            }
            return Err(AssemblerError::InvalidNumber(format!("-{}", rest)));
        }

        if is_float {
            num_str
                .parse::<f64>()
                .map(Token::Float)
                .map_err(|_| AssemblerError::InvalidNumber(num_str))
        } else {
            num_str
                .parse::<i64>()
                .map(Token::Int)
                .map_err(|_| AssemblerError::InvalidNumber(num_str))
        }
    }

    fn read_string(&mut self) -> Result<String> {
        let mut result = String::new();
        loop {
            match self.chars.next() {
                None => {
                    return Err(AssemblerError::InvalidString(
                        "Unterminated string".to_string(),
                    ));
                }
                Some((_, '"')) => break,
                Some((_, '\\')) => match self.chars.next() {
                    None => {
                        return Err(AssemblerError::InvalidString(
                            "Unterminated escape".to_string(),
                        ));
                    }
                    Some((_, 'n')) => result.push('\n'),
                    Some((_, 'r')) => result.push('\r'),
                    Some((_, 't')) => result.push('\t'),
                    Some((_, '\\')) => result.push('\\'),
                    Some((_, '"')) => result.push('"'),
                    Some((_, '0')) => result.push('\0'),
                    Some((_, 'x')) => {
                        let mut hex = String::new();
                        for _ in 0..2 {
                            match self.chars.next() {
                                Some((_, c)) if c.is_ascii_hexdigit() => hex.push(c),
                                _ => {
                                    return Err(AssemblerError::InvalidString(
                                        "Invalid hex escape".to_string(),
                                    ));
                                }
                            }
                        }
                        let byte = u8::from_str_radix(&hex, 16).map_err(|_| {
                            AssemblerError::InvalidString("Invalid hex escape".to_string())
                        })?;
                        result.push(byte as char);
                    }
                    Some((_, c)) => {
                        return Err(AssemblerError::InvalidString(format!(
                            "Unknown escape: \\{}",
                            c
                        )));
                    }
                },
                Some((_, '\n')) => {
                    self.line += 1;
                    result.push('\n');
                }
                Some((_, c)) => result.push(c),
            }
        }
        Ok(result)
    }
}

use crate::Error;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Ident,
    Lifetime,
    Literal,
    Punct,
    Eof,
}

#[derive(Clone, Copy, Debug)]
pub struct Token {
    pub kind: Kind,
    pub start: usize,
    pub end: usize,
    pub newlines_before: u32,
    pub joint: bool,
}

#[derive(Clone, Copy, Debug)]
pub struct Comment {
    pub start: usize,
    pub end: usize,
    pub line: bool,
    pub newlines_before: u32,
    pub newlines_after: u32,
}

pub struct Lexed {
    pub tokens: Vec<Token>,
    pub comments: Vec<Comment>,
    pub leading: Vec<Vec<usize>>,
    pub trailing: Vec<Vec<usize>>,
}

const PUNCTS: [&str; 22] = [
    "<<=", "...", "..=", "::", "->", "=>", "==", "!=", "<=", "&&", "||", "+=", "-=", "*=", "/=",
    "%=", "^=", "&=", "|=", "<<", "..", "#!",
];

pub const KEYWORDS: [&str; 37] = [
    "as", "break", "const", "continue", "crate", "dyn", "else", "enum", "extern", "false", "fn",
    "for", "if", "impl", "in", "let", "loop", "match", "mod", "move", "mut", "nogc", "pub", "ref",
    "return", "static", "struct", "super", "trait", "true", "type", "unsafe", "use", "where",
    "while", "async", "await",
];

fn is_ident_start(c: char) -> bool {
    c == '_' || c.is_alphabetic()
}

fn is_ident_continue(c: char) -> bool {
    c == '_' || c.is_alphanumeric()
}

struct Scanner<'s> {
    src: &'s str,
    pos: usize,
}

impl Scanner<'_> {
    fn peek(&self, n: usize) -> Option<char> {
        self.src[self.pos..].chars().nth(n)
    }

    fn rest(&self) -> &str {
        &self.src[self.pos..]
    }

    fn bump(&mut self) -> Option<char> {
        let c = self.peek(0)?;
        self.pos += c.len_utf8();
        Some(c)
    }

    fn eat_while(&mut self, f: impl Fn(char) -> bool) {
        while let Some(c) = self.peek(0) {
            if !f(c) {
                break;
            }
            self.pos += c.len_utf8();
        }
    }

    fn err(&self, offset: usize, message: &str) -> Error {
        Error::syntax(offset, message)
    }

    fn quoted(&mut self, start: usize, quote: char) -> Result<(), Error> {
        loop {
            match self.bump() {
                None => return Err(self.err(start, "unterminated literal")),
                Some('\\') => {
                    self.bump();
                }
                Some(c) if c == quote => return Ok(()),
                Some(_) => {}
            }
        }
    }

    fn raw_string(&mut self, start: usize) -> Result<(), Error> {
        let mut hashes = 0;
        while self.peek(0) == Some('#') {
            hashes += 1;
            self.pos += 1;
        }
        if self.bump() != Some('"') {
            return Err(self.err(start, "malformed raw string"));
        }
        let close = format!("\"{}", "#".repeat(hashes));
        match self.rest().find(&close) {
            Some(i) => {
                self.pos += i + close.len();
                Ok(())
            }
            None => Err(self.err(start, "unterminated raw string")),
        }
    }

    fn number(&mut self, after_dot: bool) {
        let radix = self.rest().starts_with("0x")
            || self.rest().starts_with("0o")
            || self.rest().starts_with("0b");
        if radix {
            self.pos += 2;
            self.eat_while(|c| c.is_ascii_hexdigit() || c == '_');
        } else {
            self.eat_while(|c| c.is_ascii_digit() || c == '_');
            if !after_dot
                && self.peek(0) == Some('.')
                && self.peek(1).is_some_and(|c| c.is_ascii_digit())
            {
                self.pos += 1;
                self.eat_while(|c| c.is_ascii_digit() || c == '_');
            }
            if matches!(self.peek(0), Some('e' | 'E'))
                && (self.peek(1).is_some_and(|c| c.is_ascii_digit())
                    || (matches!(self.peek(1), Some('+' | '-'))
                        && self.peek(2).is_some_and(|c| c.is_ascii_digit())))
            {
                self.pos += 2;
                self.eat_while(|c| c.is_ascii_digit() || c == '_');
            }
        }
        self.eat_while(is_ident_continue);
    }

    fn block_comment(&mut self, start: usize) -> Result<(), Error> {
        self.pos += 2;
        let mut depth = 1;
        while depth > 0 {
            if self.rest().starts_with("/*") {
                depth += 1;
                self.pos += 2;
            } else if self.rest().starts_with("*/") {
                depth -= 1;
                self.pos += 2;
            } else if self.bump().is_none() {
                return Err(self.err(start, "unterminated block comment"));
            }
        }
        Ok(())
    }
}

pub fn lex(src: &str) -> Result<Lexed, Error> {
    let mut s = Scanner { src, pos: 0 };
    let mut tokens: Vec<Token> = Vec::new();
    let mut comments: Vec<Comment> = Vec::new();
    let mut leading = Vec::new();
    let mut trailing = Vec::new();
    let mut pending: Vec<usize> = Vec::new();
    let mut newlines = 0u32;
    let mut same_line_as_token = false;
    loop {
        let start = s.pos;
        let Some(c) = s.peek(0) else { break };
        if c.is_whitespace() {
            if c == '\n' {
                newlines += 1;
                same_line_as_token = false;
            }
            s.bump();
            continue;
        }
        if s.rest().starts_with("//") || s.rest().starts_with("/*") {
            let line = s.rest().starts_with("//");
            if line {
                s.eat_while(|c| c != '\n');
            } else {
                s.block_comment(start)?;
            }
            let end = if line {
                start + src[start..s.pos].trim_end().len()
            } else {
                s.pos
            };
            let id = comments.len();
            comments.push(Comment {
                start,
                end,
                line,
                newlines_before: newlines,
                newlines_after: 0,
            });
            if same_line_as_token && newlines == 0 {
                let t: &mut Vec<usize> = trailing.last_mut().unwrap();
                t.push(id);
            } else {
                same_line_as_token = false;
                pending.push(id);
            }
            newlines = 0;
            continue;
        }
        let kind = if c == '"' {
            s.bump();
            s.quoted(start, '"')?;
            Kind::Literal
        } else if s.rest().starts_with("r\"")
            || s.rest().starts_with("r#\"")
            || s.rest().starts_with("r##")
        {
            s.bump();
            s.raw_string(start)?;
            Kind::Literal
        } else if s.rest().starts_with("br\"") || s.rest().starts_with("br#") {
            s.pos += 2;
            s.raw_string(start)?;
            Kind::Literal
        } else if s.rest().starts_with("b'") || s.rest().starts_with("b\"") {
            s.bump();
            let q = s.bump().unwrap();
            s.quoted(start, q)?;
            Kind::Literal
        } else if s.rest().starts_with("r#") && s.peek(2).is_some_and(is_ident_start) {
            s.pos += 2;
            s.eat_while(is_ident_continue);
            Kind::Ident
        } else if is_ident_start(c) {
            s.eat_while(is_ident_continue);
            Kind::Ident
        } else if c.is_ascii_digit() {
            let after_dot = tokens.last().is_some_and(|t| &src[t.start..t.end] == ".");
            s.number(after_dot);
            Kind::Literal
        } else if c == '\'' {
            if s.peek(1) == Some('\\') || s.peek(2) == Some('\'') {
                s.bump();
                s.quoted(start, '\'')?;
                Kind::Literal
            } else if s.peek(1).is_some_and(is_ident_start) {
                s.bump();
                s.eat_while(is_ident_continue);
                Kind::Lifetime
            } else {
                return Err(s.err(start, "malformed character literal"));
            }
        } else {
            match PUNCTS.iter().find(|p| s.rest().starts_with(**p)) {
                Some(p) => s.pos += p.len(),
                None => {
                    s.bump();
                }
            }
            Kind::Punct
        };
        let joint = kind == Kind::Punct
            && &src[start..s.pos] == ">"
            && matches!(s.peek(0), Some('>' | '='));
        tokens.push(Token {
            kind,
            start,
            end: s.pos,
            newlines_before: newlines,
            joint,
        });
        leading.push(std::mem::take(&mut pending));
        trailing.push(Vec::new());
        newlines = 0;
        same_line_as_token = true;
    }
    tokens.push(Token {
        kind: Kind::Eof,
        start: src.len(),
        end: src.len(),
        newlines_before: newlines,
        joint: false,
    });
    leading.push(pending);
    trailing.push(Vec::new());
    fix_newlines_after(src, &mut comments);
    Ok(Lexed {
        tokens,
        comments,
        leading,
        trailing,
    })
}

fn fix_newlines_after(src: &str, comments: &mut [Comment]) {
    for c in comments.iter_mut() {
        let rest = &src[c.end..];
        let gap = &rest[..rest
            .find(|ch: char| !ch.is_whitespace())
            .unwrap_or(rest.len())];
        c.newlines_after = gap.matches('\n').count() as u32;
    }
}

mod doc;
mod lexer;
mod parse;

use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Check {
    Reparse,
    Tokens,
    Comments,
    Idempotence,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Syntax { offset: usize, message: String },
    Internal { check: Check, offset: usize },
}

impl Error {
    pub(crate) fn syntax(offset: usize, message: impl Into<String>) -> Error {
        Error::Syntax {
            offset,
            message: message.into(),
        }
    }

    pub(crate) fn internal(check: Check, offset: usize) -> Error {
        Error::Internal { check, offset }
    }

    pub fn offset(&self) -> usize {
        match self {
            Error::Syntax { offset, .. } | Error::Internal { offset, .. } => *offset,
        }
    }

    pub fn line_col(&self, src: &str) -> (usize, usize) {
        let off = self.offset().min(src.len());
        let before = &src[..off];
        let line = before.matches('\n').count() + 1;
        let col = before.rsplit('\n').next().unwrap_or("").chars().count() + 1;
        (line, col)
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Syntax { message, .. } => write!(f, "{message}"),
            Error::Internal { check, .. } => {
                let what = match check {
                    Check::Reparse => "the formatted output does not parse",
                    Check::Tokens => "the formatted output changes the token sequence",
                    Check::Comments => "the formatted output loses or duplicates a comment",
                    Check::Idempotence => "formatting the output again changes it",
                };
                write!(
                    f,
                    "internal formatter error: {what}; the file is left unchanged"
                )
            }
        }
    }
}

impl std::error::Error for Error {}

fn layout(src: &str) -> Result<(String, Vec<(String, usize)>), Error> {
    let lexed = lexer::lex(src)?;
    let parsed = parse::parse(src, lexed)?;
    Ok((doc::print(&parsed.doc), parsed.norm))
}

fn comments(src: &str) -> Result<Vec<(&str, usize)>, Error> {
    let lexed = lexer::lex(src)?;
    let mut v: Vec<(&str, usize)> = lexed
        .comments
        .iter()
        .map(|c| (src[c.start..c.end].trim_end(), c.start))
        .collect();
    v.sort_unstable();
    Ok(v)
}

fn first_difference<T: PartialEq>(
    mut a: impl Iterator<Item = T>,
    mut b: impl Iterator<Item = T>,
) -> Option<usize> {
    let mut k = 0;
    loop {
        match (a.next(), b.next()) {
            (None, None) => return None,
            (x, y) if x == y => k += 1,
            _ => return Some(k),
        }
    }
}

fn source_offset(src: &str, out: &str, out_offset: usize) -> usize {
    let (Ok(a), Ok(b)) = (lexer::lex(src), lexer::lex(out)) else {
        return 0;
    };
    let k = b.tokens.iter().take_while(|t| t.start < out_offset).count();
    a.tokens[k.min(a.tokens.len() - 1)].start
}

pub fn format_source(src: &str) -> Result<String, Error> {
    let Some(body) = src.strip_prefix('\u{feff}') else {
        return format_body(src);
    };
    let bom = '\u{feff}'.len_utf8();
    format_body(body)
        .map(|out| format!("\u{feff}{out}"))
        .map_err(|err| match err {
            Error::Syntax { offset, message } => Error::Syntax {
                offset: offset + bom,
                message,
            },
            Error::Internal { check, offset } => Error::Internal {
                check,
                offset: offset + bom,
            },
        })
}

fn format_body(src: &str) -> Result<String, Error> {
    let (out, norm_in) = layout(src)?;
    let (again, norm_out) = layout(&out)
        .map_err(|e| Error::internal(Check::Reparse, source_offset(src, &out, e.offset())))?;
    if let Some(k) = first_difference(norm_in.iter().map(|n| &n.0), norm_out.iter().map(|n| &n.0)) {
        let offset = norm_in.get(k).map_or(src.len(), |n| n.1);
        return Err(Error::internal(Check::Tokens, offset));
    }
    let (before, after) = (comments(src)?, comments(&out)?);
    if let Some(k) = first_difference(before.iter().map(|c| c.0), after.iter().map(|c| c.0)) {
        let offset = before.get(k).map_or(src.len(), |c| c.1);
        return Err(Error::internal(Check::Comments, offset));
    }
    if let Some(i) = first_difference(out.bytes(), again.bytes()) {
        return Err(Error::internal(
            Check::Idempotence,
            source_offset(src, &out, i),
        ));
    }
    Ok(out)
}

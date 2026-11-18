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
    Compiler,
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
                    Check::Compiler => "the compiler parses the formatted output differently",
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

fn layout(src: &str) -> Result<(String, Vec<String>), Error> {
    let lexed = lexer::lex(src)?;
    let parsed = parse::parse(src, lexed)?;
    Ok((doc::print(&parsed.doc), parsed.norm))
}

fn comments(src: &str) -> Result<Vec<&str>, Error> {
    let lexed = lexer::lex(src)?;
    let mut v: Vec<&str> = lexed
        .comments
        .iter()
        .map(|c| src[c.start..c.end].trim_end())
        .collect();
    v.sort_unstable();
    Ok(v)
}

pub fn format_source(src: &str) -> Result<String, Error> {
    let (out, norm_in) = layout(src)?;
    let (again, norm_out) =
        layout(&out).map_err(|e| Error::internal(Check::Reparse, e.offset()))?;
    if norm_in != norm_out {
        return Err(Error::internal(Check::Tokens, 0));
    }
    if comments(src)? != comments(&out)? {
        return Err(Error::internal(Check::Comments, 0));
    }
    if again != out {
        let i = out
            .bytes()
            .zip(again.bytes())
            .position(|(a, b)| a != b)
            .unwrap_or(out.len().min(again.len()));
        return Err(Error::internal(Check::Idempotence, i));
    }
    compiler_agrees(src, &out)?;
    Ok(out)
}

// the stage 1 parser plugs in here: accept src, parse out, compare span-erased asts modulo r1-r4
fn compiler_agrees(_src: &str, _out: &str) -> Result<(), Error> {
    Ok(())
}

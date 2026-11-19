use aelys_common::{Diagnostic, Severity};

pub type Result<T> = std::result::Result<T, Error>;

pub struct Error {
    pub diagnostics: Vec<Diagnostic>,
    pub code: u8,
}

pub const FAILURE: u8 = 101;

impl From<Diagnostic> for Error {
    fn from(diagnostic: Diagnostic) -> Self {
        Error {
            diagnostics: vec![diagnostic],
            code: FAILURE,
        }
    }
}

pub fn error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Severity::Error, message)
}

pub fn with_help(mut diagnostic: Diagnostic, help: impl Into<String>) -> Diagnostic {
    diagnostic.add_help(help);
    diagnostic
}

pub fn io_error(context: impl std::fmt::Display, err: std::io::Error) -> Error {
    error(format!("{context}: {err}")).into()
}

pub fn closest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    candidates
        .into_iter()
        .map(|candidate| (edit_distance(word, candidate), candidate))
        .filter(|(distance, candidate)| {
            *distance <= 3 && *distance < word.len().max(candidate.len())
        })
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let substitution = diagonal + usize::from(ca != *cb);
            diagonal = row[j + 1];
            row[j + 1] = substitution.min(row[j] + 1).min(row[j + 1] + 1);
        }
    }
    row[b.len()]
}

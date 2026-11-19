use std::path::PathBuf;

pub const DEFAULT_PRELUDE: &str = "std.prelude";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceOptions {
    pub include: Vec<PathBuf>,
    pub prelude: Option<String>,
}

impl Default for SourceOptions {
    fn default() -> Self {
        Self {
            include: Vec::new(),
            prelude: Some(DEFAULT_PRELUDE.to_string()),
        }
    }
}

impl SourceOptions {
    pub fn with_include(include: Vec<PathBuf>) -> Self {
        Self {
            include,
            ..Self::default()
        }
    }
}

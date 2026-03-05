pub mod color;
pub mod registry;
pub mod render;

use aelys_syntax::{Source, Span};
use std::fmt;
use std::sync::Arc;

use self::color::ColorConfig;
use self::render::render_diagnostic;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
    Note,
    Help,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
            Severity::Note => "note",
            Severity::Help => "help",
        }
    }
}

#[derive(Debug, Clone)]
pub struct Label {
    pub source: Arc<Source>,
    pub span: Span,
    pub message: Option<String>,
    pub is_primary: bool,
}

impl Label {
    pub fn primary(source: Arc<Source>, span: Span, message: Option<String>) -> Self {
        Self {
            source,
            span,
            message,
            is_primary: true,
        }
    }

    pub fn secondary(source: Arc<Source>, span: Span, message: Option<String>) -> Self {
        Self {
            source,
            span,
            message,
            is_primary: false,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Suggestion {
    pub message: String,
    pub replacements: Vec<Replacement>,
}

#[derive(Debug, Clone)]
pub struct Replacement {
    pub span: Span,
    pub new_text: String,
    pub source: Arc<Source>,
}

#[derive(Debug, Clone)]
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Option<String>,
    pub message: String,
    pub labels: Vec<Label>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
    pub suggestions: Vec<Suggestion>,
    pub is_fatal: bool,
}

impl Diagnostic {
    pub fn new(severity: Severity, message: impl Into<String>) -> Self {
        Self {
            severity,
            code: None,
            message: message.into(),
            labels: Vec::new(),
            notes: Vec::new(),
            helps: Vec::new(),
            suggestions: Vec::new(),
            is_fatal: matches!(severity, Severity::Error),
        }
    }

    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }

    pub fn with_primary_label(
        mut self,
        source: Arc<Source>,
        span: Span,
        message: Option<String>,
    ) -> Self {
        self.labels.push(Label::primary(source, span, message));
        self
    }

    pub fn add_secondary_label(
        &mut self,
        source: Arc<Source>,
        span: Span,
        message: Option<String>,
    ) {
        self.labels.push(Label::secondary(source, span, message));
    }

    pub fn add_note(&mut self, note: impl Into<String>) {
        self.notes.push(note.into());
    }

    pub fn add_help(&mut self, help: impl Into<String>) {
        self.helps.push(help.into());
    }

    pub fn add_suggestion(&mut self, suggestion: Suggestion) {
        self.suggestions.push(suggestion);
    }

    pub fn main_label(&self) -> Option<&Label> {
        self.labels
            .iter()
            .find(|label| label.is_primary)
            .or_else(|| self.labels.first())
    }

    /// Render with explicit color configuration.
    pub fn render(&self, color: &ColorConfig) -> String {
        render_diagnostic(self, color)
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Display always renders without colors. ANSI colors are a
        // presentation concern handled by the CLI via render().
        write!(f, "{}", render_diagnostic(self, &ColorConfig::never()))
    }
}

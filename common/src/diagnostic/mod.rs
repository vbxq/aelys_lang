use aelys_syntax::{Source, Span};
use std::fmt;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    Error,
    Warning,
}

impl Severity {
    pub fn as_str(self) -> &'static str {
        match self {
            Severity::Error => "error",
            Severity::Warning => "warning",
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
pub struct Diagnostic {
    pub severity: Severity,
    pub code: Option<String>,
    pub message: String,
    pub labels: Vec<Label>,
    pub notes: Vec<String>,
    pub helps: Vec<String>,
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

    fn main_label(&self) -> Option<&Label> {
        self.labels
            .iter()
            .find(|label| label.is_primary)
            .or_else(|| self.labels.first())
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if let Some(code) = &self.code {
            writeln!(f, "{}[{}]: {}", self.severity.as_str(), code, self.message)?;
        } else {
            writeln!(f, "{}: {}", self.severity.as_str(), self.message)?;
        }

        if let Some(label) = self.main_label() {
            render_label_snippet(f, label)?;
        }

        for label in self.labels.iter().filter(|label| !label.is_primary) {
            let msg = label
                .message
                .as_deref()
                .unwrap_or("related location")
                .trim();
            writeln!(
                f,
                "   = note: {}:{}:{}: {}",
                label.source.name, label.span.line, label.span.column, msg
            )?;
        }

        for note in &self.notes {
            writeln!(f, "   = note: {}", note)?;
        }
        for help in &self.helps {
            writeln!(f, "   = help: {}", help)?;
        }

        Ok(())
    }
}

fn render_label_snippet(f: &mut fmt::Formatter<'_>, label: &Label) -> fmt::Result {
    writeln!(
        f,
        "  --> {}:{}:{}",
        label.source.name, label.span.line, label.span.column
    )?;

    let line_number = label.span.line.max(1);
    let line_content = label.source.get_line(line_number);
    let width = line_number.to_string().len().max(2);
    writeln!(f, "{:width$} |", "", width = width)?;
    writeln!(f, "{:>width$} | {}", line_number, line_content, width = width)?;

    let caret_start = label.span.column.saturating_sub(1) as usize;
    let caret_len = (label.span.end.saturating_sub(label.span.start)).max(1);
    let label_message = label.message.as_deref().unwrap_or("").trim();
    if label_message.is_empty() {
        writeln!(
            f,
            "{:width$} | {:>start$}{}",
            "",
            "",
            "^".repeat(caret_len),
            width = width,
            start = caret_start
        )?;
    } else {
        writeln!(
            f,
            "{:width$} | {:>start$}{} {}",
            "",
            "",
            "^".repeat(caret_len),
            label_message,
            width = width,
            start = caret_start
        )?;
    }

    Ok(())
}

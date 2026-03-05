use super::{CompileError, CompileErrorKind};
use std::collections::HashSet;
use std::fmt;

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut notes = Vec::new();
        let mut helps = Vec::new();
        collect_diagnostic_details(&self.kind, &mut notes, &mut helps);

        writeln!(
            f,
            "error[E{:04}]: {}",
            self.kind.code(),
            self.kind.message()
        )?;
        writeln!(
            f,
            "  --> {}:{}:{}",
            self.source.name, self.span.line, self.span.column
        )?;
        let line_content = self.source.get_line(self.span.line);
        let line_num_width = self.span.line.to_string().len().max(2);

        writeln!(f, "{:width$} |", "", width = line_num_width)?;
        writeln!(
            f,
            "{:>width$} | {}",
            self.span.line,
            line_content,
            width = line_num_width
        )?;

        let caret_start = self.span.column.saturating_sub(1) as usize;
        let caret_len = (self.span.end - self.span.start).max(1);
        let annotation = self.kind.annotation();

        writeln!(
            f,
            "{:width$} | {:>start$}{} {}",
            "",
            "",
            "^".repeat(caret_len),
            annotation,
            width = line_num_width,
            start = caret_start,
        )?;

        for note in notes {
            writeln!(f, "   = note: {}", note)?;
        }
        for help in helps {
            writeln!(f, "   = help: {}", help)?;
        }

        Ok(())
    }
}

fn collect_diagnostic_details(
    kind: &CompileErrorKind,
    notes: &mut Vec<String>,
    helps: &mut Vec<String>,
) {
    match kind {
        CompileErrorKind::TypeInferenceError(message) => {
            let mut seen = HashSet::new();
            for line in message.lines().skip(1) {
                let detail = line.trim();
                if detail.is_empty() {
                    continue;
                }
                if seen.insert(detail.to_string()) {
                    notes.push(detail.to_string());
                }
            }
        }
        CompileErrorKind::BackendDiagnostic { note, help, .. } => {
            if let Some(note) = note {
                notes.push(note.clone());
            }
            if let Some(help) = help {
                helps.push(help.clone());
            }
        }
        _ => {}
    }
}

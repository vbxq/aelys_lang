use super::Warning;
use std::fmt::{self, Write};

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let code = self.kind.code();
        let msg = self.kind.message(self.context.as_deref());

        writeln!(f, "warning[W{:04}]: {}", code, msg)?;

        if let Some(ref src) = self.source {
            writeln!(
                f,
                "  --> {}:{}:{}",
                src.name, self.span.line, self.span.column
            )?;

            let line_content = src.get_line(self.span.line);
            let width = self.span.line.to_string().len().max(2);

            writeln!(f, "{:w$} |", "", w = width)?;
            writeln!(f, "{:>w$} | {}", self.span.line, line_content, w = width)?;

            let caret_pos = self.span.column.saturating_sub(1) as usize;
            let caret_len = (self.span.end - self.span.start).max(1);
            let annotation = self.kind.annotation();

            writeln!(
                f,
                "{:w$} | {:>pos$}{} {}",
                "",
                "",
                "^".repeat(caret_len),
                annotation,
                w = width,
                pos = caret_pos
            )?;
        }

        if let Some(note) = self.kind.note() {
            writeln!(f, "   = note: {}", note)?;
        }
        if let Some(hint) = self.kind.hint() {
            writeln!(f, "   = hint: {}", hint)?;
        }

        Ok(())
    }
}

pub fn format_warnings(warnings: &[Warning]) -> String {
    let mut out = String::new();
    for (i, w) in warnings.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        let _ = write!(out, "{}", w);
    }
    out
}

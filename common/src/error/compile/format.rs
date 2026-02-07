use super::CompileError;
use std::fmt;

impl fmt::Display for CompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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

        Ok(())
    }
}

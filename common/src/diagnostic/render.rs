use super::color::ColorConfig;
use super::{Diagnostic, Label, Severity, Suggestion};

/// render  a diagnostic to a string in rustc-style format
pub fn render_diagnostic(diag: &Diagnostic, color: &ColorConfig) -> String {
    let mut out = String::new();

    // header line: error[E0401]: message
    render_header(&mut out, diag, color);
    // collect all labels, sort by line number
    let mut all_labels: Vec<&Label> = diag.labels.iter().collect();
    all_labels.sort_by_key(|l| (l.span.line, l.span.column));
    // primary labels source location: --> file:line:col
    if let Some(primary) = diag.main_label() {
        out.push_str(&format!(
            " {} {}:{}:{}\n",
            color.blue("-->"),
            primary.source.name,
            primary.span.line,
            primary.span.column,
        ));
        // determine the gutter width from all labels in the same file
        let max_line = all_labels
            .iter()
            .filter(|l| l.source.name == primary.source.name)
            .map(|l| {
                let end_line = l.source.line_col_at_offset(l.span.end).0;
                end_line.max(l.span.line)
            })
            .max()
            .unwrap_or(primary.span.line);
        let gutter_width = max_line.to_string().len().max(1);
        // group labels by line
        render_labels_block(&mut out, &all_labels, gutter_width, color);
    }

    for note in &diag.notes {
        out.push_str(&format!(
            "   {} {}\n",
            color.blue("="),
            format!("{}: {}", color.bold("note"), note),
        ));
    }

    for help in &diag.helps {
        out.push_str(&format!(
            "   {} {}\n",
            color.blue("="),
            format!(
                "{}: {}",
                color.severity(Severity::Help, "help"),
                help
            ),
        ));
    }

    // suggestion but with code diff
    for suggestion in &diag.suggestions {
        render_suggestion(&mut out, suggestion, color);
    }

    if let Some(code) = &diag.code {
        if diag.severity == Severity::Error {
            out.push_str(&format!(
                "\nFor more information about this error, try `aelys --explain {}`\n",
                code,
            ));
        }
    }

    out
}

/// render the summary line for a set of diagnostics.
pub fn render_summary(diagnostics: &[Diagnostic]) -> String {
    let error_count = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .count();
    let warning_count = diagnostics
        .iter()
        .filter(|d| d.severity == Severity::Warning)
        .count();

    let mut parts = Vec::new();
    if error_count > 0 {
        let noun = if error_count == 1 { "error" } else { "errors" };
        parts.push(format!("aborting due to {} previous {}", error_count, noun));
    }
    if warning_count > 0 {
        let noun = if warning_count == 1 {
            "warning"
        } else {
            "warnings"
        };
        parts.push(format!("{} {} emitted", warning_count, noun));
    }

    if parts.is_empty() {
        String::new()
    } else {
        format!("error: {}\n", parts.join("; "))
    }
}

fn render_header(out: &mut String, diag: &Diagnostic, color: &ColorConfig) {
    let severity_str = diag.severity.as_str();
    let header = if let Some(code) = &diag.code {
        format!(
            "{}{}{}",
            color.severity(diag.severity, severity_str),
            color.severity(diag.severity, &format!("[{}]", code)),
            color.bold(&format!(": {}", diag.message)),
        )
    } else {
        format!(
            "{}{}",
            color.severity(diag.severity, severity_str),
            color.bold(&format!(": {}", diag.message)),
        )
    };
    out.push_str(&header);
    out.push('\n');
}

fn render_labels_block(
    out: &mut String,
    labels: &[&Label],
    gutter_width: usize,
    color: &ColorConfig,
) {
    if labels.is_empty() {
        return;
    }

    let primary_source = labels
        .iter()
        .find(|l| l.is_primary)
        .or(labels.first())
        .map(|l| &l.source);

    let Some(source) = primary_source else {
        return;
    };

    // collect all lines that need to be shown
    let mut needed_lines: Vec<u32> = Vec::new();
    for label in labels {
        if label.source.name != source.name {
            continue;
        }
        let start_line = label.span.line;
        let (end_line, _) = label.source.line_col_at_offset(label.span.end.saturating_sub(1).max(label.span.start));
        for line in start_line..=end_line {
            if !needed_lines.contains(&line) {
                needed_lines.push(line);
            }
        }
    }
    needed_lines.sort();
    needed_lines.dedup();

    // empty gutter line
    out.push_str(&format!(
        "{} {}\n",
        " ".repeat(gutter_width),
        color.blue("|"),
    ));

    let mut last_line: Option<u32> = None;
    for &line_num in &needed_lines {
        // if gap show ellipsis
        if let Some(prev) = last_line {
            if line_num > prev + 1 {
                out.push_str(&format!(
                    "{}\n",
                    color.blue("..."),
                ));
            }
        }

        let line_content = source.get_line(line_num);
        let line_str = format!("{:>width$}", line_num, width = gutter_width);
        out.push_str(&format!(
            "{} {} {}\n",
            color.blue(&line_str),
            color.blue("|"),
            line_content,
        ));

        // render underlines for labels on this line
        let labels_on_line: Vec<&&Label> = labels
            .iter()
            .filter(|l| {
                l.source.name == source.name && l.span.line <= line_num && {
                    let (end_line, _) =
                        l.source.line_col_at_offset(l.span.end.saturating_sub(1).max(l.span.start));
                    end_line >= line_num
                }
            })
            .collect();

        if !labels_on_line.is_empty() {
            let mut underline = String::new();
            // build the underline string
            // we gotta compute character positions for carets
            let mut annotations: Vec<(usize, usize, bool, Option<&str>)> = Vec::new();
            for label in &labels_on_line {
                let col_start = if label.span.line == line_num {
                    label.span.column.saturating_sub(1) as usize
                } else {
                    0
                };

                let (end_line, end_col) = label.source.line_col_at_offset(
                    label.span.end.saturating_sub(1).max(label.span.start),
                );
                let col_end = if end_line == line_num {
                    end_col as usize
                } else {
                    line_content.len()
                };

                let len = col_end.saturating_sub(col_start).max(1);
                annotations.push((
                    col_start,
                    len,
                    label.is_primary,
                    label.message.as_deref(),
                ));
            }

            annotations.sort_by_key(|a| a.0);

            // render annotations (one line per annotation to avoid overlap)
            for (col_start, len, is_primary, message) in &annotations {
                let caret = if *is_primary { '^' } else { '-' };
                let carets = std::iter::repeat(caret).take(*len).collect::<String>();
                let padding = " ".repeat(*col_start);

                let msg_text = message.map(|m| m.trim()).unwrap_or("");

                let line_prefix = format!(
                    "{} {} ",
                    " ".repeat(gutter_width),
                    color.blue("|"),
                );

                if msg_text.is_empty() {
                    let caret_str = if *is_primary {
                        color.severity(Severity::Error, &carets)
                    } else {
                        color.blue(&carets)
                    };
                    underline.push_str(&format!("{}{}{}\n", line_prefix, padding, caret_str));
                } else {
                    let caret_and_msg = format!("{} {}", carets, msg_text);
                    let rendered = if *is_primary {
                        color.severity(Severity::Error, &caret_and_msg)
                    } else {
                        color.blue(&caret_and_msg)
                    };
                    underline.push_str(&format!("{}{}{}\n", line_prefix, padding, rendered));
                }
            }

            out.push_str(&underline);
        }

        last_line = Some(line_num);
    }
}

fn render_suggestion(out: &mut String, suggestion: &Suggestion, color: &ColorConfig) {
    // help: message
    out.push_str(&format!(
        "{}: {}\n",
        color.severity(Severity::Help, "help"),
        suggestion.message,
    ));

    for replacement in &suggestion.replacements {
        let line_num = replacement.span.line;
        let gutter_width = line_num.to_string().len().max(1);

        let original_line = replacement.source.get_line(line_num);
        let col_start = replacement.span.column.saturating_sub(1) as usize;
        let span_len = replacement.span.end.saturating_sub(replacement.span.start);
        let col_end = col_start + span_len;

        let mut modified_line = String::new();
        modified_line.push_str(&original_line[..col_start.min(original_line.len())]);
        modified_line.push_str(&replacement.new_text);
        if col_end < original_line.len() {
            modified_line.push_str(&original_line[col_end..]);
        }

        out.push_str(&format!(
            "{} {}\n",
            " ".repeat(gutter_width),
            color.blue("|"),
        ));
        let line_str = format!("{:>width$}", line_num, width = gutter_width);
        out.push_str(&format!(
            "{} {} {}\n",
            color.blue(&line_str),
            color.blue("|"),
            modified_line,
        ));

        // Show +++ markers under the inserted text
        let new_text_len = replacement.new_text.len();
        if new_text_len > 0 {
            let padding = " ".repeat(col_start);
            let markers = color.severity(
                Severity::Help,
                &"+".repeat(new_text_len),
            );
            out.push_str(&format!(
                "{} {} {}{}\n",
                " ".repeat(gutter_width),
                color.blue("|"),
                padding,
                markers,
            ));
        }
    }
}

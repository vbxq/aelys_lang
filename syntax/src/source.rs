use std::sync::Arc;

// source file - name + content (wrapped in Arc for cheap cloning)
#[derive(Debug, Clone)]
pub struct Source {
    pub name: String,
    pub content: String,
}

impl Source {
    pub fn new(name: impl Into<String>, content: impl Into<String>) -> Arc<Self> {
        Arc::new(Self {
            name: name.into(),
            content: content.into(),
        })
    }

    pub fn get_line(&self, n: u32) -> &str {
        self.content
            .lines()
            .nth(n.saturating_sub(1) as usize)
            .unwrap_or("")
    }

    // for error underlining - find byte offset where line n starts
    pub fn get_line_start_offset(&self, n: u32) -> usize {
        let mut off = 0;
        for (i, line) in self.content.lines().enumerate() {
            if i + 1 == n as usize {
                return off;
            }
            off += line.len() + 1;
        }
        off
    }

    /// Compute (line, column) from a byte offset. Both are 1-based.
    pub fn line_col_at_offset(&self, offset: usize) -> (u32, u32) {
        let bytes = self.content.as_bytes();
        let clamped = offset.min(bytes.len());
        let mut line = 1u32;
        let mut line_start = 0usize;

        for (index, byte) in bytes.iter().enumerate().take(clamped) {
            if *byte == b'\n' {
                line = line.saturating_add(1);
                line_start = index + 1;
            }
        }

        let column = clamped.saturating_sub(line_start).saturating_add(1) as u32;
        (line, column)
    }

    /// Get a range of source lines (1-based, inclusive). Returns (line_number, line_text) pairs.
    pub fn get_line_range(&self, start_line: u32, end_line: u32) -> Vec<(u32, &str)> {
        let start = start_line.max(1) as usize;
        let end = end_line.max(1) as usize;
        self.content
            .lines()
            .enumerate()
            .filter_map(|(i, line)| {
                let line_num = i + 1;
                if line_num >= start && line_num <= end {
                    Some((line_num as u32, line))
                } else {
                    None
                }
            })
            .collect()
    }

    /// Total number of lines in the source.
    pub fn line_count(&self) -> u32 {
        self.content.lines().count().max(1) as u32
    }
}

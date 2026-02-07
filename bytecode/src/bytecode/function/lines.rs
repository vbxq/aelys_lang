use super::Function;

impl Function {
    pub(super) fn add_line(&mut self, line: u32) {
        if let Some((count, last_line)) = self.lines.last_mut()
            && *last_line == line
            && *count < u16::MAX
        {
            *count += 1;
            return;
        }
        self.lines.push((1, line));
    }

    /// Record line info for multiple words (used for cache words after CallGlobal)
    pub fn record_lines(&mut self, count: usize, line: u32) {
        for _ in 0..count {
            self.add_line(line);
        }
    }

    /// Get line number for an instruction index
    pub fn get_line(&self, idx: usize) -> u32 {
        let mut offset = 0usize;
        for &(count, line) in &self.lines {
            offset += count as usize;
            if idx < offset {
                return line;
            }
        }
        0
    }
}

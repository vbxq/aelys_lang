use std::sync::Arc;

// source file - name + content (wrapped in Arc for cheap cloning)
#[derive(Debug, Clone)]
pub struct Source {
    pub name: String,
    pub content: String,
}

impl Source {
    pub fn new(name: impl Into<String>, content: impl Into<String>) -> Arc<Self> {
        Arc::new(Self { name: name.into(), content: content.into() })
    }

    pub fn get_line(&self, n: u32) -> &str {
        self.content.lines().nth(n.saturating_sub(1) as usize).unwrap_or("")
    }

    // for error underlining - find byte offset where line n starts
    pub fn get_line_start_offset(&self, n: u32) -> usize {
        let mut off = 0;
        for (i, line) in self.content.lines().enumerate() {
            if i + 1 == n as usize { return off; }
            off += line.len() + 1;
        }
        off
    }
}

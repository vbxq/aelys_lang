use super::Warning;
use std::fmt::{self, Write};

impl fmt::Display for Warning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.to_diagnostic())
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

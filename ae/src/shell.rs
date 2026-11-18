use std::fmt::Display;

use aelys_common::{ColorConfig, Diagnostic};

pub struct Shell {
    pub color: ColorConfig,
    quiet: bool,
}

impl Shell {
    pub fn new(color: ColorConfig, quiet: bool) -> Self {
        Shell { color, quiet }
    }

    pub fn status(&self, word: &str, message: impl Display) {
        if self.quiet {
            return;
        }
        let word = format!("{word:>12}");
        if self.color.use_color {
            eprintln!("\x1b[1;32m{word}\x1b[0m {message}");
        } else {
            eprintln!("{word} {message}");
        }
    }

    pub fn diagnostic(&self, diagnostic: &Diagnostic) {
        eprint!("{}", diagnostic.render(&self.color));
    }
}

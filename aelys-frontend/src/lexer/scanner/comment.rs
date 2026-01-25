use super::{Lexer, MAX_COMMENT_DEPTH, Result};
use aelys_common::error::{AelysError, CompileErrorKind};

impl Lexer {
    pub(super) fn block_comment(&mut self) -> Result<()> {
        let mut depth = 1;

        while depth > 0 && !self.is_at_end() {
            if self.peek() == '/' && self.peek_next() == '*' {
                self.advance();
                self.advance();
                depth += 1;
                if depth > MAX_COMMENT_DEPTH {
                    return Err(AelysError::Compile(self.error(
                        CompileErrorKind::CommentNestingTooDeep {
                            max: MAX_COMMENT_DEPTH,
                        },
                    )));
                }
            } else if self.peek() == '*' && self.peek_next() == '/' {
                self.advance();
                self.advance();
                depth -= 1;
            } else {
                if self.peek() == '\n' {
                    self.line += 1;
                    self.column = 1;
                }
                self.advance();
            }
        }

        Ok(())
    }
}

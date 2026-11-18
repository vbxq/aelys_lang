use super::Lexer;
use super::scan::is_ident_continue;
use aelys_syntax::TokenKind;

impl Lexer {
    pub(super) fn identifier(&mut self) {
        while is_ident_continue(self.peek()) {
            self.advance();
        }
        let text = self.text(self.start);
        let kind = TokenKind::keyword(text).unwrap_or_else(|| TokenKind::Identifier(text.to_string()));
        self.add_token(kind);
    }

    pub(super) fn raw_identifier(&mut self) {
        self.advance();
        let begin = self.current;
        while is_ident_continue(self.peek()) {
            self.advance();
        }
        let text = self.text(begin).to_string();
        self.add_token(TokenKind::Identifier(text));
    }
}

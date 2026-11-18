use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_syntax::{Source, TokenKind};

#[test]
fn token_spans_are_byte_offsets_and_columns_count_characters() {
    let src = "let é = \"wörld\"; let t = x;";
    let tokens = Lexer::new(src).scan().unwrap();
    for token in &tokens {
        if let TokenKind::Identifier(name) = &token.kind {
            assert_eq!(&src[token.span.start..token.span.end], name);
        }
    }
    let t = tokens
        .iter()
        .find(|t| t.kind == TokenKind::Identifier("t".to_string()))
        .unwrap();
    assert_eq!(t.span.start, src.find(" t ").unwrap() + 1);
    assert_eq!(t.span.column, 22);
}

#[test]
fn missing_semicolon_points_after_a_non_ascii_literal() {
    let src = "fn main() {\n    let s = \"é\"\n    let t = 1;\n}\n";
    let source = Source::new("t.aelys", src);
    let tokens = Lexer::with_source(source.clone()).scan().unwrap();
    let err = Parser::new(tokens, source).parse().unwrap_err();
    let aelys_common::error::AelysError::Multiple(diags) = err else {
        panic!("expected diagnostics");
    };
    let label = &diags[0].labels[0];
    assert_eq!(diags[0].code.as_deref(), Some("E0110"));
    assert_eq!(label.span.start, src.find("\"\n").unwrap() + 1);
    assert_eq!((label.span.line, label.span.column), (2, 16));
}

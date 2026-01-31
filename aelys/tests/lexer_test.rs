use aelys_frontend::lexer::Lexer;
use aelys_syntax::TokenKind;

#[test]
fn test_single_tokens() {
    let tokens = Lexer::new("( ) { } [ ] , ;").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::LParen,
            &TokenKind::RParen,
            &TokenKind::LBrace,
            &TokenKind::RBrace,
            &TokenKind::LBracket,
            &TokenKind::RBracket,
            &TokenKind::Comma,
            &TokenKind::Semicolon,
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_operators() {
    let tokens = Lexer::new("+ - * / % = == != < <= > >=").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::Plus,
            &TokenKind::Minus,
            &TokenKind::Star,
            &TokenKind::Slash,
            &TokenKind::Percent,
            &TokenKind::Eq,
            &TokenKind::EqEq,
            &TokenKind::BangEq,
            &TokenKind::Lt,
            &TokenKind::LtEq,
            &TokenKind::Gt,
            &TokenKind::GtEq,
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_integers() {
    let tokens = Lexer::new("42 0 1_000_000 0xFF 0b1010 0o755").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::Int(42),
            &TokenKind::Int(0),
            &TokenKind::Int(1_000_000),
            &TokenKind::Int(0xFF),      // 255
            &TokenKind::Int(0b1010),    // 10
            &TokenKind::Int(0o755),     // 493
            &TokenKind::Semicolon, // inserted at EOF after int
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_floats() {
    let tokens = Lexer::new("3.14 0.5 10.0").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::Float(3.14),
            &TokenKind::Float(0.5),
            &TokenKind::Float(10.0),
            &TokenKind::Semicolon, // inserted at EOF after float
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_strings() {
    let tokens = Lexer::new(r#""hello" "world" "with \"escape\"""#)
        .scan()
        .unwrap();

    assert!(matches!(&tokens[0].kind, TokenKind::String(s) if s == "hello"));
    assert!(matches!(&tokens[1].kind, TokenKind::String(s) if s == "world"));
    assert!(matches!(&tokens[2].kind, TokenKind::String(s) if s == "with \"escape\""));
}

#[test]
fn test_keywords() {
    let tokens =
        Lexer::new("let mut fn if else while return break continue and or not true false null")
            .scan()
            .unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::Let,
            &TokenKind::Mut,
            &TokenKind::Fn,
            &TokenKind::If,
            &TokenKind::Else,
            &TokenKind::While,
            &TokenKind::Return,
            &TokenKind::Break,
            &TokenKind::Continue,
            &TokenKind::And,
            &TokenKind::Or,
            &TokenKind::Not,
            &TokenKind::True,
            &TokenKind::False,
            &TokenKind::Null,
            &TokenKind::Semicolon, // inserted at EOF after null
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_identifiers() {
    let tokens = Lexer::new("foo bar _test camelCase snake_case")
        .scan()
        .unwrap();

    assert!(matches!(&tokens[0].kind, TokenKind::Identifier(s) if s == "foo"));
    assert!(matches!(&tokens[1].kind, TokenKind::Identifier(s) if s == "bar"));
    assert!(matches!(&tokens[2].kind, TokenKind::Identifier(s) if s == "_test"));
}

#[test]
fn test_line_comment() {
    let tokens = Lexer::new("a // comment\nb").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    // 'a' can end statement, newline triggers semicolon
    assert!(kinds.contains(&&TokenKind::Identifier("a".to_string())));
    assert!(kinds.contains(&&TokenKind::Semicolon));
    assert!(kinds.contains(&&TokenKind::Identifier("b".to_string())));
}

#[test]
fn test_block_comment() {
    let tokens = Lexer::new("a /* block\ncomment */ b").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert!(kinds.contains(&&TokenKind::Identifier("a".to_string())));
    assert!(kinds.contains(&&TokenKind::Identifier("b".to_string())));
    // No semicolon because newline is inside comment
}

#[test]
fn test_semicolon_after_identifier() {
    let tokens = Lexer::new("foo\nbar").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::Identifier("foo".to_string()),
            &TokenKind::Semicolon, // inserted after identifier + newline
            &TokenKind::Identifier("bar".to_string()),
            &TokenKind::Semicolon, // inserted at EOF after identifier
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_no_semicolon_after_operator() {
    let tokens = Lexer::new("a +\nb").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    // No semicolon after + because operators don't end statements
    assert_eq!(
        kinds,
        vec![
            &TokenKind::Identifier("a".to_string()),
            &TokenKind::Plus,
            &TokenKind::Identifier("b".to_string()),
            &TokenKind::Semicolon,
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_no_semicolon_after_comma() {
    let tokens = Lexer::new("foo(a,\nb)").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::Identifier("foo".to_string()),
            &TokenKind::LParen,
            &TokenKind::Identifier("a".to_string()),
            &TokenKind::Comma,
            // No semicolon here
            &TokenKind::Identifier("b".to_string()),
            &TokenKind::RParen,
            &TokenKind::Semicolon,
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_no_semicolon_after_open_brace() {
    let tokens = Lexer::new("if true {\na\n}").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::If,
            &TokenKind::True,
            &TokenKind::LBrace,
            // No semicolon after {
            &TokenKind::Identifier("a".to_string()),
            &TokenKind::Semicolon,
            &TokenKind::RBrace,
            &TokenKind::Semicolon,
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_semicolon_after_return() {
    let tokens = Lexer::new("return\nx").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    // Semicolon after return means "return null"
    assert!(kinds.contains(&&TokenKind::Return));
    assert!(kinds.contains(&&TokenKind::Semicolon));
}

#[test]
fn test_explicit_semicolon() {
    let tokens = Lexer::new("a; b; c").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::Identifier("a".to_string()),
            &TokenKind::Semicolon,
            &TokenKind::Identifier("b".to_string()),
            &TokenKind::Semicolon,
            &TokenKind::Identifier("c".to_string()),
            &TokenKind::Semicolon,
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_logical_operators_double_symbol() {
    let tokens = Lexer::new("&& ||").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::And,
            &TokenKind::Or,
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_bitwise_vs_logical() {
    let tokens = Lexer::new("& && | ||").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::Ampersand,  // single &
            &TokenKind::And,         // &&
            &TokenKind::Pipe,        // single |
            &TokenKind::Or,          // ||
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_logical_operators_in_expression() {
    let tokens = Lexer::new("true && false || true").scan().unwrap();
    let kinds: Vec<_> = tokens.iter().map(|t| &t.kind).collect();

    assert_eq!(
        kinds,
        vec![
            &TokenKind::True,
            &TokenKind::And,
            &TokenKind::False,
            &TokenKind::Or,
            &TokenKind::True,
            &TokenKind::Semicolon,
            &TokenKind::Eof,
        ]
    );
}

#[test]
fn test_logical_operators_word_and_symbol() {
    // Test that both syntaxes work and produce the same tokens
    let tokens1 = Lexer::new("and or").scan().unwrap();
    let tokens2 = Lexer::new("&& ||").scan().unwrap();

    let kinds1: Vec<_> = tokens1.iter().map(|t| &t.kind).collect();
    let kinds2: Vec<_> = tokens2.iter().map(|t| &t.kind).collect();

    assert_eq!(kinds1, kinds2);
}

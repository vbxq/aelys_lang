use aelys_frontend::lexer::Lexer;
use aelys_syntax::TokenKind;

fn kinds(src: &str) -> Vec<TokenKind> {
    Lexer::new(src).scan().unwrap().into_iter().map(|t| t.kind).collect()
}

fn ident(s: &str) -> TokenKind {
    TokenKind::Identifier(s.to_string())
}

#[test]
fn punctuation_and_operators() {
    use TokenKind::*;
    assert_eq!(
        kinds("( ) { } [ ] , ; :: -> => # ? .. ..= && || ! & | ^ << >> &= |= ^= <<= >>= += -= *= /= %= != == <= >="),
        vec![
            LParen, RParen, LBrace, RBrace, LBracket, RBracket, Comma, Semicolon, ColonColon, Arrow,
            FatArrow, Pound, Question, DotDot, DotDotEq, AndAnd, OrOr, Bang, Ampersand, Pipe, Caret,
            Shl, Shr, AmpEq, PipeEq, CaretEq, ShlEq, ShrEq, PlusEq, MinusEq, StarEq, SlashEq,
            PercentEq, BangEq, EqEq, LtEq, GtEq, Eof
        ]
    );
}

#[test]
fn newlines_are_whitespace() {
    assert_eq!(kinds("a\nb\n"), vec![ident("a"), ident("b"), TokenKind::Eof]);
    assert_eq!(kinds("return\nx"), vec![TokenKind::Return, ident("x"), TokenKind::Eof]);
}

#[test]
fn keywords_old_and_new() {
    use TokenKind::*;
    assert_eq!(
        kinds("loop use mod static move type trait where dyn ref crate super"),
        vec![Loop, Use, Mod, Static, Move, Type, Trait, Where, Dyn, Ref, Crate, Super, Eof]
    );
    assert_eq!(
        kinds("and or not needs from discard catch step null"),
        vec![
            ident("and"),
            ident("or"),
            ident("not"),
            ident("needs"),
            ident("from"),
            ident("discard"),
            ident("catch"),
            ident("step"),
            ident("null"),
            Eof
        ]
    );
    assert_eq!(kinds("r#type r#and"), vec![ident("type"), ident("and"), Eof]);
}

#[test]
fn numbers_with_suffixes() {
    use TokenKind::*;
    assert_eq!(
        kinds("42 1_000 0xFF 0b1010 0o17 200u8 7usize 1.5f32 2.5 1e3"),
        vec![
            Int(42),
            Int(1000),
            Int(255),
            Int(10),
            Int(15),
            SuffixedInt(200, "u8".to_string()),
            SuffixedInt(7, "usize".to_string()),
            SuffixedFloat(1.5, "f32".to_string()),
            Float(2.5),
            Float(1000.0),
            Eof
        ]
    );
    assert_eq!(kinds("0x1f32"), vec![Int(0x1f32), Eof]);
    assert_eq!(kinds("t.0.1"), vec![ident("t"), Dot, Int(0), Dot, Int(1), Eof]);
}

#[test]
fn strings_are_never_interpolated() {
    assert_eq!(
        kinds(r##""{name} {{x}}" r"C:\dir\{n}" r#"a "q" b"#"##),
        vec![
            TokenKind::String("{name} {{x}}".to_string()),
            TokenKind::String("C:\\dir\\{n}".to_string()),
            TokenKind::String("a \"q\" b".to_string()),
            TokenKind::Eof
        ]
    );
    assert_eq!(
        kinds("\"a\\\n     b \\x41\\u{e9}\""),
        vec![TokenKind::String("ab Aé".to_string()), TokenKind::Eof]
    );
}

#[test]
fn chars_bytes_and_lifetimes() {
    use TokenKind::*;
    assert_eq!(
        kinds("'a' '\\n' '\\u{e9}' b'A' 'outer 'ß'"),
        vec![
            Char('a' as u32),
            Char('\n' as u32),
            Char(0xe9),
            Byte(65),
            Lifetime("outer".to_string()),
            Char('ß' as u32),
            Eof
        ]
    );
}

#[test]
fn byte_strings_are_refused() {
    let err = Lexer::new("b\"abc\"").scan().unwrap_err();
    assert!(format!("{err:?}").contains("byte strings are not supported"));
}

#[test]
fn comments_vanish() {
    assert_eq!(
        kinds("a // line\n/* block /* nested */ */ b"),
        vec![ident("a"), ident("b"), TokenKind::Eof]
    );
}

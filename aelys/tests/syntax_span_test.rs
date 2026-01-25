#[test]
fn span_source_roundtrip() {
    let src = aelys_syntax::Source::new("<t>", "abc");
    let _span = aelys_syntax::Span::new(0, 3, 1, 1);
    assert_eq!(src.get_line(1), "abc");
}

use aelys::api::compile_to_typed_ast;
use aelys_air::lower::lower;
use aelys_air::print::print_program;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_syntax::Source;

fn lower_source(code: &str) -> aelys_air::AirProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().unwrap();
    let ast = Parser::new(tokens, src.clone()).parse().unwrap();
    let typed = TypeInference::infer_program(ast, src).unwrap();
    lower(&typed)
}

#[test]
fn parse_basic_enum_declaration() {
    let src = r#"
enum Color {
    Red,
    Green,
    Blue,
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "basic enum should parse: {:?}",
        result.err()
    );
}

#[test]
fn parse_enum_variant_construction() {
    let src = r#"
enum Color {
    Red,
    Green,
    Blue,
}

let c = Color::Red
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "enum construction should type-check: {:?}",
        result.err()
    );
}

#[test]
fn enum_variant_type_is_enum() {
    let src = r#"
enum Color {
    Red,
    Green,
    Blue,
}

let a = Color::Red
let b = Color::Green
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "multiple variant constructions should type-check: {:?}",
        result.err()
    );
}

#[test]
fn enum_variant_type_error_unknown_variant() {
    let src = r#"
enum Color { Red, Green, Blue }
let c = Color::Yellow
"#;
    let result = compile_to_typed_ast(src);
    assert!(result.is_err(), "unknown variant should fail type check");
}

#[test]
fn enum_variant_type_error_unknown_enum() {
    let src = r#"
let c = Bogus::Foo
"#;
    let result = compile_to_typed_ast(src);
    assert!(result.is_err(), "unknown enum should fail type check");
}

#[test]
fn enum_variants_unify_same_enum() {
    let src = r#"
enum Direction { Up, Down, Left, Right }

fn choose(flag: bool) -> Direction {
    if flag {
        return Direction::Up
    }
    return Direction::Down
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "same-enum variants should unify: {:?}",
        result.err()
    );
}

#[test]
fn enum_mismatch_different_enums() {
    let src = r#"
enum Color { Red, Green }
enum Shape { Circle, Square }

fn bad() -> Color {
    return Shape::Circle
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_err(),
        "different enum types should not unify with return type"
    );
}

#[test]
fn enum_as_function_param() {
    let src = r#"
enum Color { Red, Green, Blue }

fn paint(c: Color) -> i64 {
    return 1
}

let x = paint(Color::Red)
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "enum as function param should type-check: {:?}",
        result.err()
    );
}

// ============ Data Variant Tests ============

#[test]
fn data_variant_parse_and_typecheck() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
    Write(string),
}
let msg = Message::Write("hello")
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "data variant should type-check: {:?}",
        result.err()
    );
}

#[test]
fn data_variant_unit_still_works() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
    Write(string),
}
let msg = Message::Quit
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "unit variant in data enum should type-check: {:?}",
        result.err()
    );
}

#[test]
fn data_variant_multiple_fields() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
    Write(string),
}
let msg = Message::Move(10, 20)
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "multi-field data variant should type-check: {:?}",
        result.err()
    );
}

#[test]
fn data_variant_wrong_arg_count() {
    let src = r#"
enum Message { Quit, Move(i64, i64) }
let msg = Message::Move(1)
"#;
    let result = compile_to_typed_ast(src);
    assert!(result.is_err(), "wrong arg count should fail");
}

#[test]
fn data_variant_too_many_args() {
    let src = r#"
enum Message { Quit, Move(i64, i64) }
let msg = Message::Move(1, 2, 3)
"#;
    let result = compile_to_typed_ast(src);
    assert!(result.is_err(), "too many args should fail");
}

#[test]
fn data_variant_wrong_arg_type() {
    let src = r#"
enum Message { Write(string) }
let msg = Message::Write(42)
"#;
    let result = compile_to_typed_ast(src);
    assert!(result.is_err(), "wrong arg type should fail");
}

#[test]
fn unit_variant_with_args_fails() {
    let src = r#"
enum Color { Red, Green }
let c = Color::Red(42)
"#;
    let result = compile_to_typed_ast(src);
    assert!(result.is_err(), "unit variant with args should fail");
}

#[test]
fn data_variant_as_function_param() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
    Write(string),
}

fn handle(m: Message) -> i64 {
    return 0
}

let result = handle(Message::Write("test"))
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "data variant as function param should type-check: {:?}",
        result.err()
    );
}

#[test]
fn data_variant_return_type_unifies() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
    Write(string),
}

fn make_msg(flag: bool) -> Message {
    if flag {
        return Message::Quit
    }
    return Message::Write("hello")
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "mixed variants should unify return type: {:?}",
        result.err()
    );
}

#[test]
fn data_variant_air_lowering() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
    Write(string),
}
let msg = Message::Write("hello")
"#;
    let air = lower_source(src);
    // Verify the enum def exists in the AIR program
    let enum_def = air.enums.iter().find(|e| e.name == "Message");
    assert!(enum_def.is_some(), "Message enum should exist in AIR");
    let def = enum_def.unwrap();
    // Verify the data variant has payload types
    let write_variant = def.variants.iter().find(|v| v.name == "Write");
    assert!(write_variant.is_some(), "Write variant should exist");
    assert_eq!(
        write_variant.unwrap().payload.len(),
        1,
        "Write variant should have 1 payload field"
    );
}

#[test]
fn data_variant_unit_in_data_enum_air() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
    Write(string),
}
let msg = Message::Quit
"#;
    let air = lower_source(src);
    let enum_def = air.enums.iter().find(|e| e.name == "Message");
    assert!(enum_def.is_some(), "Message enum should exist in AIR");
    let def = enum_def.unwrap();
    let quit_variant = def.variants.iter().find(|v| v.name == "Quit");
    assert!(quit_variant.is_some(), "Quit variant should exist");
    assert!(
        quit_variant.unwrap().payload.is_empty(),
        "Quit variant should have no payload"
    );
}

#[test]
fn data_variant_multi_field_air() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
    Write(string),
}
let msg = Message::Move(10, 20)
"#;
    let air = lower_source(src);
    let enum_def = air.enums.iter().find(|e| e.name == "Message");
    assert!(enum_def.is_some(), "Message enum should exist in AIR");
    let def = enum_def.unwrap();
    let move_variant = def.variants.iter().find(|v| v.name == "Move");
    assert!(move_variant.is_some(), "Move variant should exist");
    assert_eq!(
        move_variant.unwrap().payload.len(),
        2,
        "Move variant should have 2 payload fields"
    );
}

#[test]
fn simple_enum_still_works_after_data_variant_changes() {
    let src = r#"
enum Color { Red, Green, Blue }
let c = Color::Red
"#;
    let air = lower_source(src);
    let enum_def = air.enums.iter().find(|e| e.name == "Color");
    assert!(enum_def.is_some(), "Color enum should exist in AIR");
    let def = enum_def.unwrap();
    // All variants should have empty payload (simple enum)
    for v in &def.variants {
        assert!(
            v.payload.is_empty(),
            "simple enum variant {} should have no payload",
            v.name
        );
    }
}

#[test]
fn data_variant_bool_field() {
    let src = r#"
enum Event {
    Click(i64, i64),
    Toggle(bool),
}
let e = Event::Toggle(true)
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "bool field data variant should type-check: {:?}",
        result.err()
    );
}

#[test]
fn data_variant_with_mixed_alignment_fields() {
    let src = r#"
enum Message {
    Quit,
    Mixed(i64, bool, i64),
}
let msg = Message::Mixed(1, true, 3)
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "data variant with mixed alignment should type-check: {:?}",
        result.err()
    );
}

#[test]
fn data_variant_mixed_types() {
    let src = r#"
enum Data {
    IntVal(i64),
    FloatVal(f64),
    StrVal(string),
    BoolVal(bool),
}
let d1 = Data::IntVal(42)
let d2 = Data::FloatVal(3.14)
let d3 = Data::StrVal("hello")
let d4 = Data::BoolVal(false)
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "mixed-type data variants should type-check: {:?}",
        result.err()
    );
}

// ============ Match Expression Tests ============

#[test]
fn match_simple_enum_exhaustive() {
    let src = r#"
enum Color { Red, Green, Blue }

fn name(c: Color) -> i64 {
    return match c {
        Color::Red => 1,
        Color::Green => 2,
        Color::Blue => 3,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "exhaustive match on simple enum should type-check: {:?}",
        result.err()
    );
}

#[test]
fn match_with_wildcard() {
    let src = r#"
enum Color { Red, Green, Blue }

fn name(c: Color) -> i64 {
    return match c {
        Color::Red => 1,
        _ => 0,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "match with wildcard should type-check: {:?}",
        result.err()
    );
}

#[test]
fn match_non_exhaustive_error() {
    let src = r#"
enum Color { Red, Green, Blue }

fn name(c: Color) -> i64 {
    return match c {
        Color::Red => 1,
        Color::Green => 2,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_err(),
        "non-exhaustive match should fail type check"
    );
}

#[test]
fn match_duplicate_variant_error() {
    let src = r#"
enum Color { Red, Green, Blue }

fn name(c: Color) -> i64 {
    return match c {
        Color::Red => 1,
        Color::Red => 2,
        Color::Green => 3,
        Color::Blue => 4,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_err(),
        "duplicate variant pattern should fail type check"
    );
}

#[test]
fn match_wrong_enum_in_pattern() {
    let src = r#"
enum Color { Red, Green, Blue }
enum Shape { Circle, Square }

fn name(c: Color) -> i64 {
    return match c {
        Shape::Circle => 1,
        _ => 0,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_err(),
        "wrong enum in pattern should fail type check"
    );
}

#[test]
fn match_unknown_variant_error() {
    let src = r#"
enum Color { Red, Green, Blue }

fn name(c: Color) -> i64 {
    return match c {
        Color::Yellow => 1,
        _ => 0,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_err(),
        "unknown variant in pattern should fail type check"
    );
}

#[test]
fn match_data_variant_with_bindings() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
    Write(string),
}

fn handle(m: Message) -> i64 {
    return match m {
        Message::Quit => 0,
        Message::Move(x, y) => x,
        Message::Write(text) => 1,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "match with data variant bindings should type-check: {:?}",
        result.err()
    );
}

#[test]
fn match_data_variant_wrong_binding_count() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
}

fn handle(m: Message) -> i64 {
    return match m {
        Message::Quit => 0,
        Message::Move(x) => x,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_err(),
        "wrong binding count should fail type check"
    );
}

#[test]
fn match_as_expression_in_let() {
    let src = r#"
enum Color { Red, Green, Blue }

fn test(c: Color) -> i64 {
    let x = match c {
        Color::Red => 10,
        Color::Green => 20,
        Color::Blue => 30,
    }
    return x
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "match as expression in let should type-check: {:?}",
        result.err()
    );
}

#[test]
fn match_arms_type_mismatch() {
    let src = r#"
enum Color { Red, Green, Blue }

fn test(c: Color) -> i64 {
    return match c {
        Color::Red => 1,
        Color::Green => "hello",
        Color::Blue => 3,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_err(),
        "match arms with different types should fail"
    );
}

#[test]
fn match_simple_enum_air_lowering() {
    let src = r#"
enum Color { Red, Green, Blue }

fn name(c: Color) -> i64 {
    return match c {
        Color::Red => 1,
        Color::Green => 2,
        Color::Blue => 3,
    }
}
"#;
    let air = lower_source(src);
    let air_text = print_program(&air);

    // The AIR should contain a switch terminator
    assert!(
        air_text.contains("switch"),
        "match should lower to switch terminator, got:\n{}",
        air_text
    );
    // The AIR should contain enum_tag extraction
    assert!(
        air_text.contains("enum_tag"),
        "match should extract enum tag, got:\n{}",
        air_text
    );
}

#[test]
fn match_data_variant_air_lowering() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
    Write(string),
}

fn handle(m: Message) -> i64 {
    return match m {
        Message::Quit => 0,
        Message::Move(x, y) => x,
        Message::Write(text) => 1,
    }
}
"#;
    let air = lower_source(src);
    let air_text = print_program(&air);

    // Should contain switch, enum_tag, and enum_payload
    assert!(
        air_text.contains("switch"),
        "match should lower to switch, got:\n{}",
        air_text
    );
    assert!(
        air_text.contains("enum_tag"),
        "match should extract tag, got:\n{}",
        air_text
    );
    assert!(
        air_text.contains("enum_payload"),
        "data match should extract payload, got:\n{}",
        air_text
    );
}

#[test]
fn match_with_wildcard_air_lowering() {
    let src = r#"
enum Color { Red, Green, Blue }

fn name(c: Color) -> i64 {
    return match c {
        Color::Red => 1,
        _ => 0,
    }
}
"#;
    let air = lower_source(src);
    let air_text = print_program(&air);

    // Should contain switch with default block
    assert!(
        air_text.contains("switch"),
        "match with wildcard should lower to switch, got:\n{}",
        air_text
    );
    assert!(
        air_text.contains("default"),
        "match with wildcard should have default block, got:\n{}",
        air_text
    );
}

#[test]
fn match_all_unit_variants_exhaustive_no_default() {
    let src = r#"
enum Dir { Up, Down }

fn check(d: Dir) -> i64 {
    return match d {
        Dir::Up => 1,
        Dir::Down => 2,
    }
}
"#;
    let air = lower_source(src);
    let air_text = print_program(&air);

    // Should contain switch and unreachable (since it is exhaustive without wildcard)
    assert!(
        air_text.contains("switch"),
        "exhaustive match should use switch, got:\n{}",
        air_text
    );
    assert!(
        air_text.contains("unreachable"),
        "exhaustive match without wildcard should have unreachable default, got:\n{}",
        air_text
    );
}

#[test]
fn match_binding_types_are_correct() {
    let src = r#"
enum Message {
    Quit,
    Move(i64, i64),
}

fn get_x(m: Message) -> i64 {
    return match m {
        Message::Quit => 0,
        Message::Move(x, y) => x,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "binding types should propagate correctly: {:?}",
        result.err()
    );
}

#[test]
fn match_nested_in_if() {
    let src = r#"
enum Color { Red, Green, Blue }

fn test(c: Color, flag: bool) -> i64 {
    if flag {
        return match c {
            Color::Red => 1,
            Color::Green => 2,
            Color::Blue => 3,
        }
    }
    return 0
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "match nested in if should type-check: {:?}",
        result.err()
    );
}

#[test]
fn match_in_function_return() {
    let src = r#"
enum Color { Red, Green, Blue }

fn to_int(c: Color) -> i64 {
    return match c {
        Color::Red => 0,
        Color::Green => 1,
        Color::Blue => 2,
    }
}

let r = to_int(Color::Green)
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "match in function return should type-check: {:?}",
        result.err()
    );
}

#[test]
fn match_with_semicolons_between_arms() {
    // The parser should accept both commas and semicolons between arms
    let src = r#"
enum Color { Red, Green, Blue }

fn test(c: Color) -> i64 {
    return match c {
        Color::Red => 1
        Color::Green => 2
        Color::Blue => 3
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "match with auto-semicolons should parse: {:?}",
        result.err()
    );
}

#[test]
fn match_single_arm_wildcard() {
    let src = r#"
enum Color { Red, Green, Blue }

fn test(c: Color) -> i64 {
    return match c {
        _ => 42,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "match with only wildcard should type-check: {:?}",
        result.err()
    );
}

#[test]
fn match_data_variant_uses_binding_in_body() {
    // Verify that bindings are actually usable in the arm body
    let src = r#"
enum Wrapper {
    Val(i64),
    None,
}

fn extract(w: Wrapper) -> i64 {
    return match w {
        Wrapper::Val(x) => x,
        Wrapper::None => 0,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "binding used in arm body should type-check: {:?}",
        result.err()
    );
    // Also verify AIR lowering works
    let air = lower_source(src);
    let air_text = print_program(&air);
    assert!(
        air_text.contains("enum_payload"),
        "should extract payload for binding, got:\n{}",
        air_text
    );
}

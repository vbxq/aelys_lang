use aelys::api::compile_to_typed_ast;
use aelys_air::lower::lower;
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
    assert!(result.is_ok(), "basic enum should parse: {:?}", result.err());
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
    assert_eq!(write_variant.unwrap().payload.len(), 1, "Write variant should have 1 payload field");
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
    assert!(quit_variant.unwrap().payload.is_empty(), "Quit variant should have no payload");
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
    assert_eq!(move_variant.unwrap().payload.len(), 2, "Move variant should have 2 payload fields");
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
        assert!(v.payload.is_empty(), "simple enum variant {} should have no payload", v.name);
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
    assert!(result.is_ok(), "data variant with mixed alignment should type-check: {:?}", result.err());
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

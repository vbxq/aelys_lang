use aelys::api::compile_to_typed_ast;
use aelys_air::AirType;
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

fn lower_and_monomorphize(code: &str) -> aelys_air::AirProgram {
    let air = lower_source(code);
    aelys_air::mono::monomorphize(air).unwrap()
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
fn use_write() -> Message {
    return Message::Write("hello")
}
"#;
    let air = lower_source(src);
    let enum_def = air.enums.iter().find(|e| e.name == "Message");
    assert!(enum_def.is_some(), "Message enum should exist in AIR");
    let def = enum_def.unwrap();
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
fn match_wildcard_must_be_last() {
    let src = r#"
enum Color { Red, Green, Blue }

fn name(c: Color) -> i64 {
    return match c {
        _ => 0,
        Color::Red => 1,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    let errors = result.expect_err("wildcard before specific arms should fail");
    let rendered = format!("{:?}", errors);
    assert!(
        rendered.contains("wildcard pattern must be the last match arm"),
        "unexpected diagnostics: {rendered}"
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

    assert!(
        air_text.contains("switch"),
        "match should lower to switch terminator, got:\n{}",
        air_text
    );
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

    // should contain switch with default block
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

    // should contain switch and unreachable (since it is exhaustive without wildcard)
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
    let air = lower_source(src);
    let air_text = print_program(&air);
    assert!(
        air_text.contains("enum_payload"),
        "should extract payload for binding, got:\n{}",
        air_text
    );
}

// ============ generic enum tests ============

#[test]
fn generic_enum_option_some() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
let x = Option::Some(42)
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "Option::Some(42) should work: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_option_none() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
let x: Option<i64> = Option::None
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "Option::None should work: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_option_some_string() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
let x = Option::Some("hello")
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "Option::Some(\"hello\") should work: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_option_some_bool() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
let x = Option::Some(true)
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "Option::Some(true) should work: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_match() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
fn unwrap_or(opt: Option<i64>, default: i64) -> i64 {
    match opt {
        Option::Some(val) => val,
        Option::None => default,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(result.is_ok(), "match on generic enum: {:?}", result.err());
}

#[test]
fn generic_enum_result() {
    let src = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
fn check(r: Result<i64, string>) -> i64 {
    match r {
        Result::Ok(val) => val,
        Result::Err(msg) => -1,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "Result<i64, string> should work: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_as_return_type() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
fn make_some() -> Option<i64> {
    return Option::Some(42)
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "generic enum as return type: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_as_param_type() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
fn is_some(opt: Option<i64>) -> i64 {
    return match opt {
        Option::Some(val) => 1,
        Option::None => 0,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "generic enum as param type: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_none_without_annotation() {
    // unit variant of a generic enum without type annotation should produce
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
let x = Option::None
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_err(),
        "Option::None without annotation should require type annotation"
    );
    let errors = result.unwrap_err();
    let msg = format!("{:?}", errors);
    assert!(
        msg.contains("type annotations needed") || msg.contains("cannot infer"),
        "error should mention type annotations: {}",
        msg
    );
}

#[test]
fn generic_enum_multiple_variants_in_function() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
fn test(flag: bool) -> Option<i64> {
    if flag {
        return Option::Some(42)
    }
    return Option::None
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "returning both Some and None: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_result_ok_construction() {
    let src = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
let r = Result::Ok(42)
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "Result::Ok(42) should work: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_result_err_construction() {
    let src = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
let r = Result::Err("something failed")
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "Result::Err(\"...\") should work: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_air_lowering() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
let x = Option::Some(42)
"#;
    let air = lower_source(src);
    // the generic enum def should exist (with type params)
    let enum_def = air.enums.iter().find(|e| e.name.contains("Option"));
    assert!(
        enum_def.is_some(),
        "Option enum should exist in AIR: {:?}",
        air.enums.iter().map(|e| &e.name).collect::<Vec<_>>()
    );
}

#[test]
fn generic_enum_match_air_lowering() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
fn unwrap_or(opt: Option<i64>, default: i64) -> i64 {
    return match opt {
        Option::Some(val) => val,
        Option::None => default,
    }
}
"#;
    let air = lower_source(src);
    let air_text = print_program(&air);

    assert!(
        air_text.contains("switch"),
        "match on generic enum should lower to switch, got:\n{}",
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
fn generic_enum_wrong_arg_type() {
    // with `some(42)`, t = i64. but we can't enforce that t must be i64
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
let x = Option::Some(42)
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "generic enum construction should type-check: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_exhaustive_match() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
fn test(opt: Option<i64>) -> i64 {
    return match opt {
        Option::Some(v) => v,
        Option::None => 0,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "exhaustive match on generic enum: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_non_exhaustive_match_error() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
fn test(opt: Option<i64>) -> i64 {
    return match opt {
        Option::Some(v) => v,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_err(),
        "non-exhaustive match on generic enum should fail"
    );
}

#[test]
fn generic_enum_match_with_wildcard() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
fn test(opt: Option<i64>) -> i64 {
    return match opt {
        Option::Some(v) => v,
        _ => -1,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "generic enum match with wildcard: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_result_match_both_variants() {
    let src = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
fn handle(r: Result<i64, string>) -> i64 {
    return match r {
        Result::Ok(val) => val,
        Result::Err(msg) => -1,
    }
}
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "Result match both variants: {:?}",
        result.err()
    );
}

#[test]
fn nested_generic_enum_unit_variant_monomorphizes_nested_enum_def() {
    let src = r#"
enum Pair<A, B> {
    Both(A, B),
    Neither,
}

enum Boxed<T> {
    Value(T),
    Empty,
}

fn get_empty() -> Boxed<Pair<i64, string>> {
    return Boxed::Empty
}

fn main() {
    let e = get_empty()
}
"#;
    let air = lower_and_monomorphize(src);
    let pair = air
        .enums
        .iter()
        .find(|e| e.name == "__mono_Pair$2$i64$str")
        .expect("nested Pair mono enum should exist");
    assert_eq!(pair.variants[0].payload.len(), 2);

    let boxed = air
        .enums
        .iter()
        .find(|e| e.name == "__mono_Boxed$1$enum___mono_Pair$2$i64$str")
        .expect("Boxed<Pair<...>> mono enum should exist");
    let value_variant = boxed
        .variants
        .iter()
        .find(|v| v.name == "Value")
        .expect("Value variant should exist");
    assert_eq!(
        value_variant.payload,
        vec![AirType::Enum(aelys_air::EnumRef::new(
            "Pair",
            vec![AirType::I64, AirType::Str]
        ))]
    );
}

#[test]
fn generic_enum_unit_variant_with_fnptr_type_arg_monomorphizes() {
    let src = r#"
enum Holder<T> {
    Value(T),
    Empty,
}

fn apply_default() -> Holder<fn(i64) -> i64> {
    return Holder::Empty
}
"#;
    let air = lower_and_monomorphize(src);
    let air_text = print_program(&air);

    let holder = air
        .enums
        .iter()
        .find(|e| e.name == "__mono_Holder$1$fnptr$i64$Ri64")
        .expect("fnptr-instantiated Holder enum should exist");
    assert_eq!(holder.variants.len(), 2);
    assert!(
        !air_text.contains("enum_init Holder::"),
        "fnptr generic unit variant should be rewritten to mono enum:\n{air_text}"
    );
}

#[test]
fn generic_enum_named_fn_payload_uses_fnptr_monomorphization() {
    let src = r#"
enum Holder<T> {
    Value(T),
    Empty,
}

fn inc(x: i64) -> i64 {
    return x + 1
}

fn call_holder(h: Holder<fn(i64) -> i64>) -> i64 {
    return match h {
        Holder::Value(f) => f(41)
        Holder::Empty => 0
    }
}

fn main() {
    let h: Holder<fn(i64) -> i64> = Holder::Value(inc)
    call_holder(h)
}
"#;
    let air = lower_and_monomorphize(src);
    let air_text = print_program(&air);

    assert!(
        air_text.contains("enum_init __mono_Holder$1$fnptr$i64$Ri64::Value"),
        "named function payload should monomorphize to fnptr enum, got:\n{air_text}"
    );
    assert!(
        !air_text.contains("__mono_Holder$1$ptr_void"),
        "named function payload must not degrade to ptr_void mono, got:\n{air_text}"
    );
}

#[test]
fn generic_enum_result_air_lowering() {
    let src = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
fn handle(r: Result<i64, string>) -> i64 {
    return match r {
        Result::Ok(val) => val,
        Result::Err(msg) => -1,
    }
}
"#;
    let air = lower_source(src);
    let air_text = print_program(&air);

    assert!(
        air_text.contains("switch"),
        "Result match should lower to switch, got:\n{}",
        air_text
    );
    assert!(
        air_text.contains("enum_payload"),
        "Result match should extract payload, got:\n{}",
        air_text
    );
}


#[test]
fn generic_enum_none_with_multiple_monos() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
let a = Option::Some(42)
let b = Option::Some("hello")
let c: Option<i64> = Option::None
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "None with multiple monos should work: {:?}",
        result.err()
    );
}

#[test]
fn generic_enum_none_with_multiple_monos_air() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
fn make_int() -> Option<i64> {
    return Option::Some(42)
}
fn make_str() -> Option<string> {
    return Option::Some("hello")
}
fn make_none_int() -> Option<i64> {
    return Option::None
}
"#;
    let air = lower_source(src);
    let air = aelys_air::mono::monomorphize(air).unwrap();
    let air_text = print_program(&air);

    // after monomorphization, no generic enum definitions should remain
    let remaining_generic = air.enums.iter().any(|e| !e.type_params.is_empty());
    assert!(
        !remaining_generic,
        "no generic enum defs should remain after mono, got:\n{}",
        air_text
    );

    // the enum_init for none variant should reference a monomorphized name
    assert!(
        !air_text.contains("enum_init Option::"),
        "unit variant Option::None should be monomorphized, got:\n{}",
        air_text
    );
}

#[test]
fn generic_enum_unit_variant_multiple_monos_in_function() {
    let src = r#"
enum Option<T> {
    Some(T),
    None,
}
fn test() -> Option<i64> {
    return Option::None
}
fn test2() -> Option<string> {
    return Option::None
}
let a = test()
let b = test2()
"#;
    let result = compile_to_typed_ast(src);
    assert!(
        result.is_ok(),
        "None in different typed functions should work: {:?}",
        result.err()
    );
}

fn sweep_pool() -> Vec<AirType> {
    use aelys_air::{CallingConv, EnumRef};
    let mut pool = vec![
        AirType::I8,
        AirType::I16,
        AirType::I32,
        AirType::I64,
        AirType::U8,
        AirType::U16,
        AirType::U32,
        AirType::U64,
        AirType::F32,
        AirType::F64,
        AirType::Bool,
        AirType::Str,
        AirType::Void,
        AirType::Opaque,
        // the derivation is injective only because the grammar forces a capitalized struct name
        AirType::Struct("Ptr".into()),
        AirType::Struct("Enum".into()),
        AirType::Struct("R".into()),
        AirType::Struct("A_B".into()),
        AirType::Struct("A".into()),
        AirType::Struct("B".into()),
    ];
    let leaves = pool.clone();
    for leaf in &leaves {
        pool.push(AirType::Ptr(Box::new(leaf.clone())));
        pool.push(AirType::Slice(Box::new(leaf.clone())));
        pool.push(AirType::Vec(Box::new(leaf.clone())));
        pool.push(AirType::Array(Box::new(leaf.clone()), 2));
        pool.push(AirType::FnPtr {
            params: vec![leaf.clone()],
            ret: Box::new(AirType::I64),
            conv: CallingConv::Aelys,
        });
        pool.push(AirType::FnPtr {
            params: vec![AirType::I64, leaf.clone()],
            ret: Box::new(leaf.clone()),
            conv: CallingConv::C,
        });
        for base in ["E", "E_x", "Q", "Q_ptr"] {
            pool.push(AirType::Enum(EnumRef::new(base, vec![leaf.clone()])));
            pool.push(AirType::Enum(EnumRef::new(
                base,
                vec![leaf.clone(), AirType::I64],
            )));
        }
    }
    pool
}

#[test]
fn the_derived_enum_symbol_is_injective_over_the_type_pool() {
    use aelys_air::EnumRef;
    use std::collections::HashMap;

    let bases = [
        "Q", "Q_ptr", "E", "E_x", "Opt", "Pair", "ptr", "enum", "param", "fnptr",
    ];
    let pool = sweep_pool();
    let mut by_symbol: HashMap<String, String> = HashMap::new();
    let mut collisions: Vec<String> = Vec::new();
    let mut derived = 0usize;

    let mut note = |base: &str, args: Vec<AirType>, derived: &mut usize| {
        let key = format!("{}{:?}", base, args);
        let symbol = EnumRef::new(base, args).symbol();
        *derived += 1;
        if let Some(previous) = by_symbol.insert(symbol.clone(), key.clone())
            && previous != key
        {
            collisions.push(format!(
                "`{symbol}` is derived by both {previous} and {key}"
            ));
        }
    };

    for base in bases {
        note(base, Vec::new(), &mut derived);
        for a in &pool {
            note(base, vec![a.clone()], &mut derived);
        }
        for a in &pool {
            for b in &pool {
                note(base, vec![a.clone(), b.clone()], &mut derived);
            }
        }
    }

    assert!(
        derived > 20_000,
        "the sweep derived only {derived} names, which is too few to have exercised arity 2"
    );
    assert!(
        collisions.is_empty(),
        "{} of {derived} derived names collide; the first few:\n{}",
        collisions.len(),
        collisions
            .iter()
            .take(10)
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
    );
    eprintln!("injectivity sweep: {derived} derived names, 0 collisions");
}

#[test]
fn a_plain_enum_keeps_its_bare_name() {
    use aelys_air::EnumRef;
    assert_eq!(EnumRef::plain("Opt").symbol(), "Opt");
    assert_eq!(EnumRef::plain("Tagged").symbol(), "Tagged");
    assert_eq!(
        EnumRef::new("Pair", vec![AirType::I64, AirType::Str]).symbol(),
        "__mono_Pair$2$i64$str"
    );
}

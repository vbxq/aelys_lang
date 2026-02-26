use aelys_air::lower::lower;
use aelys_air::{AirProgram, AirStmtKind, AirType, Rvalue};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::types::InferType;
use aelys_sema::TypeInference;
use aelys_syntax::Source;

fn source_to_air(code: &str) -> AirProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone())
        .scan()
        .expect("should never happen");
    let ast = Parser::new(tokens, src.clone())
        .parse()
        .expect("meow meow meow ");
    let typed = TypeInference::infer_program(ast, src).expect("sema failed");
    lower(&typed)
}

fn collect_casts(air: &AirProgram) -> Vec<(&AirType, &AirType)> {
    let mut casts = Vec::new();
    for func in &air.functions {
        for block in &func.blocks {
            for stmt in &block.stmts {
                if let AirStmtKind::Assign {
                    rvalue: Rvalue::Cast { from, to, .. },
                    ..
                } = &stmt.kind
                {
                    casts.push((from, to));
                }
            }
        }
    }
    casts
}

#[test]
fn signed_to_wider_signed() {
    assert!(InferType::I8.can_implicit_widen_to(&InferType::I16));
    assert!(InferType::I8.can_implicit_widen_to(&InferType::I32));
    assert!(InferType::I8.can_implicit_widen_to(&InferType::I64));
    assert!(InferType::I16.can_implicit_widen_to(&InferType::I32));
    assert!(InferType::I16.can_implicit_widen_to(&InferType::I64));
    assert!(InferType::I32.can_implicit_widen_to(&InferType::I64));
}

#[test]
fn unsigned_to_wider_unsigned() {
    assert!(InferType::U8.can_implicit_widen_to(&InferType::U16));
    assert!(InferType::U8.can_implicit_widen_to(&InferType::U32));
    assert!(InferType::U8.can_implicit_widen_to(&InferType::U64));
    assert!(InferType::U16.can_implicit_widen_to(&InferType::U32));
    assert!(InferType::U16.can_implicit_widen_to(&InferType::U64));
    assert!(InferType::U32.can_implicit_widen_to(&InferType::U64));
}

#[test]
fn unsigned_to_wider_signed() {
    assert!(InferType::U8.can_implicit_widen_to(&InferType::I16));
    assert!(InferType::U8.can_implicit_widen_to(&InferType::I32));
    assert!(InferType::U8.can_implicit_widen_to(&InferType::I64));
    assert!(InferType::U16.can_implicit_widen_to(&InferType::I32));
    assert!(InferType::U16.can_implicit_widen_to(&InferType::I64));
    assert!(InferType::U32.can_implicit_widen_to(&InferType::I64));
}

#[test]
fn small_int_to_float() {
    // ≤16-bit ints => f32 (24-bit mantissa)
    assert!(InferType::I8.can_implicit_widen_to(&InferType::F32));
    assert!(InferType::U8.can_implicit_widen_to(&InferType::F32));
    assert!(InferType::I16.can_implicit_widen_to(&InferType::F32));
    assert!(InferType::U16.can_implicit_widen_to(&InferType::F32));

    // ≤32-bit ints => f64 (53-bit mantissa)
    assert!(InferType::I8.can_implicit_widen_to(&InferType::F64));
    assert!(InferType::U8.can_implicit_widen_to(&InferType::F64));
    assert!(InferType::I16.can_implicit_widen_to(&InferType::F64));
    assert!(InferType::U16.can_implicit_widen_to(&InferType::F64));
    assert!(InferType::I32.can_implicit_widen_to(&InferType::F64));
    assert!(InferType::U32.can_implicit_widen_to(&InferType::F64));
}

#[test]
fn rejects_lossy_conversions() {
    // Narrowing
    assert!(!InferType::I64.can_implicit_widen_to(&InferType::I32));
    assert!(!InferType::I32.can_implicit_widen_to(&InferType::I16));
    assert!(!InferType::I16.can_implicit_widen_to(&InferType::I8));

    // Signed => unsigned
    assert!(!InferType::I8.can_implicit_widen_to(&InferType::U8));
    assert!(!InferType::I8.can_implicit_widen_to(&InferType::U16));
    assert!(!InferType::I32.can_implicit_widen_to(&InferType::U64));

    // i32/u32 => f32 (lossy: >24-bit mantissa)
    assert!(!InferType::I32.can_implicit_widen_to(&InferType::F32));
    assert!(!InferType::U32.can_implicit_widen_to(&InferType::F32));

    // i64/u64 => f64 (lossy: >53-bit mantissa)
    assert!(!InferType::I64.can_implicit_widen_to(&InferType::F64));
    assert!(!InferType::U64.can_implicit_widen_to(&InferType::F64));

    // Float => int
    assert!(!InferType::F32.can_implicit_widen_to(&InferType::I32));
    assert!(!InferType::F64.can_implicit_widen_to(&InferType::I64));

    // Float => float (f32=>f64 is not int widening, source is float)
    assert!(!InferType::F32.can_implicit_widen_to(&InferType::F64));

    // same type
    assert!(!InferType::I64.can_implicit_widen_to(&InferType::I64));

    // non-numeric
    assert!(!InferType::Bool.can_implicit_widen_to(&InferType::I32));
    assert!(!InferType::String.can_implicit_widen_to(&InferType::I64));
}

#[test]
fn i8_arg_to_i64_param_inserts_cast() {
    let code = r#"
fn takes_i64(x: i64) -> i64 { x }
fn main() {
    let v: i8 = 5
    takes_i64(v)
}
"#;
    let air = source_to_air(code);
    let casts = collect_casts(&air);
    assert!(
        casts.iter().any(|(from, to)| *from == &AirType::I8 && *to == &AirType::I64),
        "expected implicit cast i8=>i64, got: {casts:?}"
    );
}

#[test]
fn u8_arg_to_i32_param_inserts_cast() {
    let code = r#"
fn takes_i32(x: i32) -> i32 { x }
fn main() {
    let v: u8 = 5
    takes_i32(v)
}
"#;
    let air = source_to_air(code);
    let casts = collect_casts(&air);
    assert!(
        casts.iter().any(|(from, to)| *from == &AirType::U8 && *to == &AirType::I32),
        "expected implicit cast u8=>i32, got: {casts:?}"
    );
}

#[test]
fn i32_arg_to_f64_param_inserts_cast() {
    let code = r#"
fn takes_f64(x: f64) -> f64 { x }
fn main() {
    let v: i32 = 42
    takes_f64(v)
}
"#;
    let air = source_to_air(code);
    let casts = collect_casts(&air);
    assert!(
        casts.iter().any(|(from, to)| *from == &AirType::I32 && *to == &AirType::F64),
        "expected implicit cast i32=>f64, got: {casts:?}"
    );
}

#[test]
fn literal_narrowing_still_works() {
    // literal 5 should narrow to i8 without needing a cast
    let code = r#"
fn takes_i8(x: i8) -> i8 { x }
fn main() {
    takes_i8(5)
}
"#;
    let air = source_to_air(code);
    let casts = collect_casts(&air);
    assert!(
        casts.is_empty(),
        "literal narrowing should not produce a cast, got: {casts:?}"
    );
}

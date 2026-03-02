/// Stress tests for literal narrowing edge cases.
/// These probe boundaries the happy-path tests never touch.
use aelys_driver::compile_file_with_llvm;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::OptimizationLevel;
use aelys_sema::TypeInference;
use aelys_syntax::Source;
use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use std::fs;
use tempfile::tempdir;

fn sema_ok(code: &str) -> bool {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    TypeInference::infer_program(stmts, src).is_ok()
}

fn compile_to_verified_ir(source: &str) -> String {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, source).expect("source should be written");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect("llvm backend compilation should succeed");
    let ll_path = source_path.with_extension("ll");
    let ir = fs::read_to_string(&ll_path).expect("llvm ir file should be generated");
    let context = Context::create();
    let buffer = MemoryBuffer::create_from_file(&ll_path).expect("llvm ir should be readable");
    let module = context
        .create_module_from_ir(buffer)
        .expect("llvm ir should parse into a module");
    module
        .verify()
        .expect("module.verify() should succeed for generated ir");
    ir
}

#[test]
fn return_i32_literal_zero() {
    assert!(
        sema_ok("fn f() -> i32 { return 0 }"),
        "return 0 in i32 fn should pass sema"
    );
}

#[test]
fn return_i32_literal_positive() {
    assert!(
        sema_ok("fn f() -> i32 { return 42 }"),
        "return 42 in i32 fn should pass sema"
    );
}

#[test]
fn return_i32_literal_max() {
    assert!(
        sema_ok("fn f() -> i32 { return 2147483647 }"),
        "return i32::MAX should pass sema"
    );
}

#[test]
fn return_i32_literal_overflow() {
    assert!(
        !sema_ok("fn f() -> i32 { return 2147483648 }"),
        "return i32::MAX+1 should FAIL sema"
    );
}

#[test]
fn return_i8_literal() {
    assert!(
        sema_ok("fn f() -> i8 { return 127 }"),
        "return 127 in i8 fn should pass sema"
    );
}

#[test]
fn return_i8_literal_overflow() {
    assert!(
        !sema_ok("fn f() -> i8 { return 128 }"),
        "return 128 in i8 fn should FAIL sema"
    );
}

#[test]
fn return_u8_literal() {
    assert!(
        sema_ok("fn f() -> u8 { return 255 }"),
        "return 255 in u8 fn should pass sema"
    );
}

#[test]
fn return_u8_literal_overflow() {
    assert!(
        !sema_ok("fn f() -> u8 { return 256 }"),
        "return 256 in u8 fn should FAIL sema"
    );
}

#[test]
fn return_negative_i32() {
    // -1 is parsed as Unary(Neg, Int(1)) not as Int(-1). try_narrow_literal only matches TypedExprKind::Int so this will not be narrowed, producing Mismatch{I64, I32}.
    let ok = sema_ok("fn f() -> i32 { return -1 }");
    assert!(
        ok,
        "return -1 in i32 fn should pass sema (narrowing bug if this fails)"
    );
}

#[test]
fn return_negative_i8() {
    let ok = sema_ok("fn f() -> i8 { return -1 }");
    assert!(
        ok,
        "return -1 in i8 fn should pass sema (narrowing BUG if this fails)"
    );
}

#[test]
fn return_binop_i32() {
    // 1 + 2 -> both default to I64, result is I64. Return type is I32.
    // no narrowing for BinOp results
    let ok = sema_ok("fn f() -> i32 { return 1 + 2 }");
    assert!(
        ok,
        "return 1+2 in i32 fn should pass sema (binop narrowing BUG if this fails)"
    );
}

#[test]
fn return_i32_param_plus_literal() {
    // a is i32 param, 1 defaults to i64 -> binop constraint i32 == i64 --> fail ?
    let ok = sema_ok("fn f(a: i32) -> i32 { return a + 1 }");
    assert!(
        ok,
        "i32 param + literal should work (binop litteral bug if this fails)"
    );
}

#[test]
fn let_i32_annotation_literal() {
    let ok = sema_ok("fn f() { let x: i32 = 42 }");
    assert!(
        ok,
        "let x: i32 = 42 should pass sema (let narrowing bug if this fails)"
    );
}

#[test]
fn let_i8_annotation_literal() {
    let ok = sema_ok("fn f() { let x: i8 = 10 }");
    assert!(
        ok,
        "let x: i8 = 10 should pass sema (let narrowing bug if this fails)"
    );
}

#[test]
fn array_i32_annotation_with_literals() {
    let ok = sema_ok("fn f() { let arr: [i32; 3] = [1, 2, 3] }");
    assert!(ok, "annotated i32 array with literals should pass sema");
}

#[test]
fn if_else_both_return_i32_literal() {
    let ok = sema_ok(
        r#"
fn f(x: bool) -> i32 {
    if x {
        return 1
    }
    return 0
}
"#,
    );
    assert!(ok, "both branches returning i32 literals should pass sema");
}

#[test]
fn if_else_return_i32_from_nested() {
    let ok = sema_ok(
        r#"
fn f(x: bool, y: bool) -> i32 {
    if x {
        if y {
            return 1
        }
        return 2
    }
    return 3
}
"#,
    );
    assert!(ok, "nested if returns with i32 literals should pass sema");
}

#[test]
fn codegen_i32_return_literal() {
    let ir = compile_to_verified_ir("fn f() -> i32 { return 42 }");
    assert!(
        ir.contains("ret i32"),
        "i32 function should return i32, not i64:\n{ir}"
    );
}

#[test]
fn codegen_i32_if_else_returns() {
    let ir = compile_to_verified_ir(
        r#"
fn f(x: i32) -> i32 {
    if x > 0 {
        return 1
    }
    return 0
}
"#,
    );
    // every ret in this function should be i32
    for line in ir.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("ret ") && !trimmed.starts_with("ret void") {
            assert!(
                trimmed.contains("ret i32"),
                "expected ret i32 but got: {trimmed}\nFull IR:\n{ir}"
            );
        }
    }
}

#[test]
fn return_f32_literal() {
    let ok = sema_ok("fn f() -> f32 { return 1.0 }");
    assert!(ok, "return 1.0 in f32 fn should pass sema");
}

#[test]
fn return_f32_literal_codegen() {
    let ir = compile_to_verified_ir("fn f() -> f32 { return 1.0 }");
    assert!(
        ir.contains("ret float"),
        "f32 function should return float:\n{ir}"
    );
}

#[test]
fn assign_i32_literal() {
    // let x: i32 = 0; x = 42 assign.rs does not call try_narrow_literal
    let ok = sema_ok(
        r#"
fn f() {
    let mut x: i32 = 0
    x = 42
}
"#,
    );
    assert!(
        ok,
        "x = 42 where x: i32 should pass sema (ASSIGNMENT NARROWING BUG if fails)"
    );
}

#[test]
fn assign_i8_literal() {
    let ok = sema_ok(
        r#"
fn f() {
    let mut x: i8 = 0
    x = 10
}
"#,
    );
    assert!(
        ok,
        "x = 10 where x: i8 should pass sema (ASSIGNMENT NARROWING BUG if fails)"
    );
}

#[test]
fn struct_field_i32_literal() {
    // StructLiteral pushes constraint I64 == I32 without narrowing
    let ok = sema_ok(
        r#"
struct Foo { x: i32 }
fn f() { let a = Foo { x: 42 } }
"#,
    );
    assert!(ok, "struct field i32 init with literal should pass sema");
}

#[test]
fn struct_field_i8_literal() {
    let ok = sema_ok(
        r#"
struct Bar { val: i8 }
fn f() { let b = Bar { val: 10 } }
"#,
    );
    assert!(ok, "struct field i8 init with literal should pass sema");
}

#[test]
fn implicit_return_i32_literal() {
    // implicit return constraint in implicit.rs does not narrow
    let ok = sema_ok("fn f() -> i32 { 42 }");
    assert!(ok, "implicit return 42 in i32 fn should pass sema");
}

#[test]
fn index_assign_i32_array_literal() {
    let ok = sema_ok(
        r#"
fn f() {
    let arr: [i32; 3] = [0, 0, 0]
    arr[0] = 42
}
"#,
    );
    assert!(ok, "index assign to i32 array should pass sema");
}

#[test]
fn return_binop_chain_i32() {
    let ok = sema_ok("fn f() -> i32 { return 1 + 2 + 3 }");
    assert!(
        ok,
        "return 1+2+3 in i32 fn should pass sema (binop chain bug if fails)"
    );
}

#[test]
fn return_i32_param_binop_literal_codegen() {
    // this should work because narrow_binop_int_literals narrows the literal to match the param type
    let ir = compile_to_verified_ir(
        r#"
fn add_one(x: i32) -> i32 {
    return x + 1
}
"#,
    );
    assert!(ir.contains("add i32"), "should add i32 not i64:\n{ir}");
}

#[test]
fn while_loop_i32_counter() {
    let ok = sema_ok(
        r#"
fn f() -> i32 {
    let mut i: i32 = 0
    while i < 10 {
        i = i + 1
    }
    return i
}
"#,
    );
    assert!(ok, "while loop with i32 counter should pass sema");
}

#[test]
fn comparison_i32_with_literal() {
    // i < 10 where i: i32 and 10 defaults to i64 narrow_binop_int_literals should handle this
    let ok = sema_ok(
        r#"
fn f(x: i32) -> bool {
    return x > 0
}
"#,
    );
    assert!(
        ok,
        "i32 > 0 should pass sema (literal should narrow to i32)"
    );
}

#[test]
fn binary_literal_plus_non_narrowable_should_reject() {
    // `1 + x` where x is i64 should not be silently narrowed to i32.
    // before, try_narrow_literal returned true for identifiers, causing the binary expression to be incorrectly retyped to i32.
    let ok = sema_ok(
        r#"
fn f(x: i64) -> i32 {
    return 1 + x
}
"#,
    );
    assert!(
        !ok,
        "return (1 + x:i64) as i32 must FAIL sema, non-narrowable operand should block binary narrowing"
    );
}

#[test]
fn binary_non_narrowable_plus_literal_should_reject() {
    // Same as above but with operands reversed
    let ok = sema_ok(
        r#"
fn f(x: i64) -> i32 {
    return x + 1
}
"#,
    );
    assert!(
        !ok,
        "return (x:i64 + 1) as i32 must FAIL sema, non-narrowable operand should block binary narrowing"
    );
}

#[test]
fn binary_both_literals_still_narrows() {
    // pure literal binary expressions should still narrow successfully
    let ok = sema_ok("fn f() -> i32 { return 1 + 2 }");
    assert!(
        ok,
        "return (1 + 2) in i32 fn should still pass sema"
    );
}

#[test]
fn binary_literal_plus_same_type_param_works() {
    // a:i32 + 1 should work because narrow_binop_int_literals narrows 1 to i32, so both operands are i32 before try_narrow_literal is ever called from return context
    let ok = sema_ok(
        r#"
fn f(a: i32) -> i32 {
    return a + 1
}
"#,
    );
    assert!(
        ok,
        "return (a:i32 + 1) as i32 should pass sema, narrow_binop_int_literals handles this"
    );
}

#[test]
fn unary_non_narrowable_should_not_corrupt() {
    // unary on a non-narrowable should not succeed in narrowing -x where x is i64 should not be silently narrowed to i32
    let ok = sema_ok(
        r#"
fn negate(x: i64) -> i32 {
    return -x
}
"#,
    );
    assert!(
        !ok,
        "return (-x:i64) as i32 must fail sema, non-narrowable in unary should block narrowing"
    );
}

#[test]
fn binary_add_overflow_i8_must_fail() {
    // 100 + 100 = 200, which overflows i8 (-128..127).
    let ok = sema_ok("fn f() -> i8 { return 100 + 100 }");
    assert!(
        !ok,
        "return (100 + 100) as i8 must fail sema, result 200 overflows i8"
    );
}

#[test]
fn binary_add_within_i8_must_pass() {
    // 50 + 50 = 100, which fits in i8 (-128..127).
    let ok = sema_ok("fn f() -> i8 { return 50 + 50 }");
    assert!(
        ok,
        "return (50 + 50) as i8 must pass sema, result 100 fits in i8"
    );
}

#[test]
fn binary_mul_overflow_i8_must_fail() {
    // 100 * 2 = 200, overflows i8.
    let ok = sema_ok("fn f() -> i8 { return 100 * 2 }");
    assert!(
        !ok,
        "return (100 * 2) as i8 must fail sema, result 200 overflows i8"
    );
}

#[test]
fn binary_sub_overflow_i8_must_fail() {
    let ok = sema_ok("fn f() -> i8 { return 127 + 1 }");
    assert!(
        !ok,
        "return (127 + 1) as i8 must fail sema, result 128 overflows i8"
    );
}

#[test]
fn binary_sub_within_i8_must_pass() {
    let ok = sema_ok("fn f() -> i8 { return 100 - 50 }");
    assert!(
        ok,
        "return (100 - 50) as i8 must pass sema, result 50 fits in i8"
    );
}

#[test]
fn binary_add_overflow_i16_must_fail() {
    // 30000 + 30000 = 60000, overflows i16 (-32768..32767).
    let ok = sema_ok("fn f() -> i16 { return 30000 + 30000 }");
    assert!(
        !ok,
        "return (30000 + 30000) as i16 must fail sema, result 60000 overflows i16"
    );
}

#[test]
fn binary_add_within_i16_must_pass() {
    let ok = sema_ok("fn f() -> i16 { return 10000 + 10000 }");
    assert!(
        ok,
        "return (10000 + 10000) as i16 must pass sema, result 20000 fits in i16"
    );
}

#[test]
fn binary_mul_within_i32_must_pass() {
    let ok = sema_ok("fn f() -> i32 { return 1000 * 1000 }");
    assert!(
        ok,
        "return (1000 * 1000) as i32 must pass sema, result fits in i32"
    );
}

#[test]
fn struct_field_i32_narrowed_literal_with_constraint() {
    // Struct field init with a narrowable literal. Narrowing changes 64 from
    // i64 to i32, and the solver must also see the Equal(i32, i32) constraint
    // to validate.
    let ok = sema_ok(
        r#"
struct Pair { a: i32, b: i32 }
fn f() {
    let p = Pair { a: 1, b: 2 }
}
"#,
    );
    assert!(
        ok,
        "struct field init with narrowable i32 literals must pass sema"
    );
}

#[test]
fn struct_field_i8_narrowed_literal_with_constraint() {
    // same as above but with i8 to exercise smaller integer narrowing
    let ok = sema_ok(
        r#"
struct Small { val: i8 }
fn f() {
    let s = Small { val: 100 }
}
"#,
    );
    assert!(
        ok,
        "struct field init with narrowable i8 literal must pass sema"
    );
}

#[test]
fn let_i32_annotation_literal_with_constraint() {
    // let with type annotation and literal, narrowing changes 42 from i64
    // to i32, and the solver must see the Equal(i32, i32) constraint too3.
    // before, the constraint was skipped when narrowing succeeded
    let ok = sema_ok(
        r#"
fn f() {
    let x: i32 = 42
    let y: i32 = x + 1
}
"#,
    );
    assert!(
        ok,
        "let x: i32 = 42 followed by use must pass sema"
    );
}

#[test]
fn let_i8_annotation_literal_with_constraint() {
    let ok = sema_ok(
        r#"
fn f() {
    let x: i8 = 10
}
"#,
    );
    assert!(
        ok,
        "let x: i8 = 10 must pass sema"
    );
}

#[test]
fn struct_field_string_where_i32_expected_rejected() {
    let ok = sema_ok(
        r#"
struct Typed { val: i32 }
fn f() {
    let t = Typed { val: "wrong" }
}
"#,
    );
    assert!(
        !ok,
        "struct field with string where i32 expected must fail sema"
    );
}

#[test]
fn let_string_where_i64_expected_rejected() {
    // solver catches the type mismatch that narrowing cannot fix.
    let ok = sema_ok(
        r#"
fn f() {
    let x: i64 = "bad"
}
"#,
    );
    assert!(
        !ok,
        "let x: i64 = \"bad\" must fail sema, solver catches the mismatch"
    );
}

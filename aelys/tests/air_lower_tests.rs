use aelys_air::lower::lower;
use aelys_air::*;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_syntax::Source;

fn lower_source(code: &str) -> AirProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().unwrap();
    let ast = Parser::new(tokens, src.clone()).parse().unwrap();
    let typed = TypeInference::infer_program(ast, src).unwrap();
    lower(&typed)
}

fn func<'a>(air: &'a AirProgram, name: &str) -> &'a AirFunction {
    air.functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("function '{name}' not found"))
}

fn has_named_call(f: &AirFunction, target: &str) -> bool {
    f.blocks
        .iter()
        .flat_map(|b| &b.stmts)
        .any(|s| match &s.kind {
            AirStmtKind::Assign {
                rvalue:
                    Rvalue::Call {
                        func: Callee::Named(n),
                        ..
                    },
                ..
            } => n == target,
            AirStmtKind::CallVoid {
                func: Callee::Named(n),
                ..
            } => n == target,
            _ => false,
        })
}

#[test]
fn closure_env_struct_has_captured_field() {
    let air = lower_source("fn outer(x) {\n    return fn(y) { return x + y }\n}");
    let env = air
        .structs
        .iter()
        .find(|s| s.is_closure_env)
        .expect("no closure env struct");
    assert!(env.fields.iter().any(|f| f.name == "x"));
}

#[test]
fn closure_function_first_param_is_env_ptr() {
    let air = lower_source("fn outer(x) {\n    return fn(y) { return x + y }\n}");
    let lambda = air
        .functions
        .iter()
        .find(|f| f.name.starts_with("__lambda"))
        .expect("no lambda function");
    let first = &lambda.params[0];
    assert!(matches!(
        &first.ty,
        AirType::Ptr(inner) if matches!(
            inner.as_ref(),
            AirType::Struct(s) if s.starts_with("__closure_env")
        )
    ));
}

#[test]
fn while_loop_has_branch_and_back_edge() {
    let air = lower_source(
        "fn f() {\n    let mut x = 0\n    while x < 10 {\n        x = x + 1\n    }\n}",
    );
    let f = func(&air, "f");
    assert!(f.blocks.len() >= 3);
    let header = f
        .blocks
        .iter()
        .find(|b| matches!(b.terminator, AirTerminator::Branch { .. }))
        .expect("no Branch terminator");
    assert!(
        f.blocks
            .iter()
            .any(|b| matches!(b.terminator, AirTerminator::Goto(t) if t == header.id)),
        "no back-edge to header"
    );
}

#[test]
fn for_loop_has_four_blocks_with_mutable_iterator() {
    let air = lower_source("fn f() {\n    for i in 0..10 {\n        let x = i\n    }\n}");
    let f = func(&air, "f");
    assert!(f.blocks.len() >= 4);
    let iter_local = f
        .locals
        .iter()
        .find(|l| l.name.as_deref() == Some("i"))
        .expect("iterator 'i' not found");
    assert!(iter_local.is_mut);
    let header = f
        .blocks
        .iter()
        .find(|b| matches!(b.terminator, AirTerminator::Branch { .. }))
        .expect("no Branch in for loop");
    let gotos_to_header = f
        .blocks
        .iter()
        .filter(|b| matches!(b.terminator, AirTerminator::Goto(t) if t == header.id))
        .count();
    assert!(gotos_to_header >= 2, "expected entry + incr Goto to header");
}

#[test]
fn short_circuit_and_produces_branch() {
    let air = lower_source("fn f() {\n    let x = true && false\n}");
    let f = func(&air, "f");
    assert!(f.blocks.len() >= 3);
    assert!(
        f.blocks
            .iter()
            .any(|b| matches!(b.terminator, AirTerminator::Branch { .. }))
    );
}

#[test]
fn fmt_string_calls_to_string_and_concat() {
    let air = lower_source("fn f() {\n    let s = \"value: {42}\"\n}");
    let f = func(&air, "f");
    assert!(has_named_call(f, "__aelys_to_string"));
    assert!(has_named_call(f, "__aelys_str_concat"));
}

#[test]
fn cast_produces_rvalue_with_target_type() {
    let air = lower_source("fn f() {\n    42 as f64\n}");
    let f = func(&air, "f");
    let has_cast = f.blocks.iter().flat_map(|b| &b.stmts).any(|s| {
        matches!(
            &s.kind,
            AirStmtKind::Assign { rvalue: Rvalue::Cast { to, .. }, .. } if *to == AirType::F64
        )
    });
    assert!(has_cast);
}

#[test]
fn default_gc_mode_is_managed() {
    let air = lower_source("fn f() { }");
    assert_eq!(func(&air, "f").gc_mode, GcMode::Managed);
}

#[test]
fn no_gc_decorator_sets_manual_mode() {
    let air = lower_source("@no_gc\nfn f() { }");
    assert_eq!(func(&air, "f").gc_mode, GcMode::Manual);
}

#[test]
fn int_literal_defaults_to_i64() {
    let air = lower_source("let a = 42");
    let g = air
        .globals
        .iter()
        .find(|g| g.name == "a")
        .expect("global 'a' not found");
    assert!(matches!(g.init, Some(AirConst::Int(42, AirIntSize::I64))));
}

#[test]
fn float_literal_defaults_to_f64() {
    let air = lower_source("let pi = 3.14");
    let g = air
        .globals
        .iter()
        .find(|g| g.name == "pi")
        .expect("global 'pi' not found");
    assert!(matches!(
        g.init,
        Some(AirConst::Float(_, AirFloatSize::F64))
    ));
}

#[test]
fn top_level_global_read_uses_global_get_instead_of_closure_env() {
    let air = lower_source(
        r#"
let g = 7

fn main() -> i64 {
    return g
}
"#,
    );

    let main = func(&air, "main");
    assert!(
        main.params.is_empty(),
        "main should not get a hidden env param"
    );
    assert!(
        !air.structs.iter().any(|s| s.name == "__closure_env_main"),
        "top-level globals must not synthesize a closure env"
    );
    assert!(
        has_named_call(main, "__aelys_global_get_g"),
        "global reads should lower to the synthetic global getter call"
    );
}

#[test]
fn top_level_simple_enum_global_lowers_to_const_tag() {
    let air = lower_source(
        r#"
enum Color {
    Red,
    Green,
}

let c: Color = Color::Green
"#,
    );

    let global = air
        .globals
        .iter()
        .find(|g| g.name == "c")
        .expect("global 'c' not found");
    assert!(matches!(
        global.init,
        Some(AirConst::Int(1, AirIntSize::I32))
    ));
}

#[test]
fn top_level_data_enum_unit_global_lowers_to_const_tag() {
    let air = lower_source(
        r#"
enum Option<T> {
    Some(T),
    None,
}

let g: Option<i64> = Option::None
"#,
    );

    let global = air
        .globals
        .iter()
        .find(|g| g.name == "g")
        .expect("global 'g' not found");
    assert!(matches!(
        global.init,
        Some(AirConst::Int(1, AirIntSize::I32))
    ));
}

#[test]
fn top_level_data_enum_payload_global_lowers_to_const_enum() {
    let air = lower_source(
        r#"
enum Option<T> {
    Some(T),
    None,
}

let g: Option<i64> = Option::Some(42)
"#,
    );

    let global = air
        .globals
        .iter()
        .find(|g| g.name == "g")
        .expect("global 'g' not found");
    assert!(matches!(
        global.init,
        Some(AirConst::Enum { ref enum_ref, tag: 0, ref payload })
            if enum_ref.symbol() == "__mono_Option$1$i64"
                && matches!(payload.as_slice(), [AirConst::Int(42, AirIntSize::I64)])
    ));
}

#[test]
fn top_level_fnptr_global_alias_lowers_to_target_fnref() {
    let air = lower_source(
        r#"
let g: fn() -> i64 = main
let h: fn() -> i64 = g

fn main() -> i64 {
    return h()
}
"#,
    );

    let global = air
        .globals
        .iter()
        .find(|g| g.name == "h")
        .expect("global 'h' not found");
    assert!(matches!(
        global.init,
        Some(AirConst::FnRef(ref name)) if name == "main"
    ));
}

#[test]
fn top_level_data_enum_global_alias_clones_const_initializer() {
    let air = lower_source(
        r#"
enum Option<T> {
    Some(T),
    None,
}

let g: Option<i64> = Option::None
let h: Option<i64> = g
"#,
    );

    let global = air
        .globals
        .iter()
        .find(|g| g.name == "h")
        .expect("global 'h' not found");
    assert!(matches!(
        global.init,
        Some(AirConst::Int(1, AirIntSize::I32))
    ));
}


#[test]
fn null_literal_lowers_to_ptr_void_not_bare_void() {
    let air = lower_source(
        r#"
fn make_null() {
    let x = null
}
"#,
    );
    let f = func(&air, "make_null");
    let null_local = f
        .locals
        .iter()
        .find(|l| l.name.as_deref() == Some("x"))
        .expect("local 'x' not found");
    assert_eq!(
        null_local.ty,
        AirType::Ptr(Box::new(AirType::Void)),
        "null literal should produce Ptr(Void), not bare Void"
    );
}

#[test]
fn null_literal_type_is_not_void() {
    // ensure the local for a null-typed variable is not airtype::void
    let air = lower_source(
        r#"
fn test_null() {
    let y = null
}
"#,
    );
    let f = func(&air, "test_null");
    let local = f
        .locals
        .iter()
        .find(|l| l.name.as_deref() == Some("y"))
        .expect("local 'y' not found");
    assert_ne!(
        local.ty,
        AirType::Void,
        "null-typed local must not be bare Void (would cause 0-byte alloca)"
    );
}


#[test]
#[should_panic(expected = "AIR lowering failed")]
fn non_constant_array_size_in_expr_produces_error() {
    lower_source(
        r#"
fn f(n: i64) -> i64 {
    let arr = [0; n]
    return 0
}
"#,
    );
}

#[test]
#[should_panic(expected = "non-constant array size")]
fn non_constant_array_size_error_message_is_descriptive() {
    lower_source(
        r#"
fn g(size: i64) -> i64 {
    let arr = [42; size]
    return 0
}
"#,
    );
}

#[test]
#[should_panic(expected = "stack array too large")]
fn oversized_stack_array_in_expr_produces_error() {
    lower_source(
        r#"
fn h() -> i64 {
    let x = [0; 200000]
    return x[0]
}
"#,
    );
}

#[test]
#[should_panic(expected = "AIR lowering failed")]
fn oversized_array_error_aggregated_in_finish() {
    lower_source(
        r#"
fn big() -> i64 {
    let huge = [1; 300000]
    return huge[0]
}
"#,
    );
}

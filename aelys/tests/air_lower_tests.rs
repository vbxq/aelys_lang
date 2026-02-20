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
    f.blocks.iter().flat_map(|b| &b.stmts).any(|s| match &s.kind {
        AirStmtKind::Assign {
            rvalue: Rvalue::Call { func: Callee::Named(n), .. },
            ..
        } => n == target,
        AirStmtKind::CallVoid { func: Callee::Named(n), .. } => n == target,
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
    assert!(f
        .blocks
        .iter()
        .any(|b| matches!(b.terminator, AirTerminator::Branch { .. })));
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
    let g = air.globals.iter().find(|g| g.name == "a").expect("global 'a' not found");
    assert!(matches!(g.init, Some(AirConst::Int(42, AirIntSize::I64))));
}

#[test]
fn float_literal_defaults_to_f64() {
    let air = lower_source("let pi = 3.14");
    let g = air.globals.iter().find(|g| g.name == "pi").expect("global 'pi' not found");
    assert!(matches!(g.init, Some(AirConst::Float(_, AirFloatSize::F64))));
}

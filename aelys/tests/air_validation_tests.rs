use aelys_air::*;
use aelys_air::layout::compute_layouts;
use aelys_air::lower::{lower, lower_with_gc_mode};
use aelys_air::mono::monomorphize;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_syntax::Source;
use std::collections::HashSet;

fn lower_source(code: &str) -> AirProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone()).parse().expect("parse failed");
    let typed = TypeInference::infer_program(stmts, src).expect("sema failed");
    lower(&typed)
}

fn lower_with_globals(code: &str, globals: &[&str]) -> AirProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone()).parse().expect("parse failed");
    let known: HashSet<String> = globals.iter().map(|s| s.to_string()).collect();
    let typed =
        TypeInference::infer_program_with_imports(stmts, src, HashSet::new(), known)
            .expect("sema failed");
    lower(&typed)
}

fn func<'a>(air: &'a AirProgram, name: &str) -> &'a AirFunction {
    air.functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("function `{}` not found in AIR", name))
}


#[test]
fn stdlib_call_print() {
    let air = lower_with_globals(
        r#"
fn greet() {
    print("hello")
}
"#,
        &["print", "println"],
    );

    let f = func(&air, "greet");

    let has_print_call = f.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| matches!(&s.kind,
            AirStmtKind::CallVoid { func: Callee::Named(n), .. }
            | AirStmtKind::Assign { rvalue: Rvalue::Call { func: Callee::Named(n), .. }, .. }
            if n == "print"
        ))
    });
    assert!(has_print_call, "expected a call to `print`");

    assert!(
        !air.structs.iter().any(|s| s.name.contains("__closure_env")),
        "no closure env struct expected for a plain function"
    );
}

#[test]
fn int_literal_narrowing() {
    let air = lower_source(
        r#"
fn take_i32(x: i32) {
    return
}
fn take_i16(y: i16) {
    return
}
fn caller() {
    take_i32(2)
    take_i16(2)
}
"#,
    );

    let f = func(&air, "caller");

    let mut call_args: Vec<&Operand> = Vec::new();
    for block in &f.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                AirStmtKind::Assign {
                    rvalue: Rvalue::Call { args, .. },
                    ..
                }
                | AirStmtKind::CallVoid { args, .. } => {
                    call_args.extend(args.iter());
                }
                _ => {}
            }
        }
    }

    let has_i32_arg = call_args.iter().any(|op| {
        matches!(op, Operand::Const(AirConst::Int(2, AirIntSize::I32)))
    });
    let has_i16_arg = call_args.iter().any(|op| {
        matches!(op, Operand::Const(AirConst::Int(2, AirIntSize::I16)))
    });
    assert!(has_i32_arg, "expected Int(2, I32) argument for take_i32 call");
    assert!(has_i16_arg, "expected Int(2, I16) argument for take_i16 call");
}

#[test]
fn overflow_literal_rejected() {
    let code = r#"
fn small(x: i8) {
    return
}
fn caller() {
    small(300)
}
"#;
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone()).parse().expect("parse failed");
    let result = TypeInference::infer_program(stmts, src);
    assert!(result.is_err(), "expected sema to reject overflow literal 300 for i8");
}

#[test]
fn nested_closure_no_globals_in_env() {
    let air = lower_with_globals(
        r#"
fn outer() {
    let x = 10
    let inner = fn() {
        print(x)
    }
}
"#,
        &["print", "println"],
    );

    let env = air
        .structs
        .iter()
        .find(|s| s.is_closure_env)
        .expect("expected a closure env struct");

    let field_names: Vec<&str> = env.fields.iter().map(|f| f.name.as_str()).collect();
    assert!(
        field_names.contains(&"x"),
        "closure env should capture `x`, got: {:?}",
        field_names
    );
    assert!(
        !field_names.contains(&"print"),
        "closure env should NOT capture global `print`, got: {:?}",
        field_names
    );
    assert!(
        !field_names.contains(&"println"),
        "closure env should NOT capture global `println`, got: {:?}",
        field_names
    );
}

#[test]
fn struct_field_access() {
    let air = lower_source(
        r#"
struct Point { x: i64, y: i64 }
fn get_x(p: Point) -> i64 {
    return p.x
}
"#,
    );

    let f = func(&air, "get_x");

    let has_field_access = f.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| matches!(&s.kind,
            AirStmtKind::Assign { rvalue: Rvalue::FieldAccess { field, .. }, .. }
            if field == "x"
        ))
    });
    assert!(has_field_access, "expected Rvalue::FieldAccess {{ field: \"x\" }}");

    let field_local = f.blocks.iter().flat_map(|b| b.stmts.iter()).find_map(|s| {
        if let AirStmtKind::Assign {
            place: Place::Local(id),
            rvalue: Rvalue::FieldAccess { field, .. },
        } = &s.kind
        {
            if field == "x" { Some(*id) } else { None }
        } else {
            None
        }
    });
    if let Some(lid) = field_local {
        let local = f.locals.iter().find(|l| l.id == lid).expect("local not found");
        assert_eq!(local.ty, AirType::I64, "field access result local should be I64");
    }
}

#[test]
fn struct_init() {
    let air = lower_source(
        r#"
struct Point { x: i64, y: i64 }
fn make() -> Point {
    return Point { x: 1, y: 2 }
}
"#,
    );

    let f = func(&air, "make");

    let init = f
        .blocks
        .iter()
        .flat_map(|b| b.stmts.iter())
        .find_map(|s| {
            if let AirStmtKind::Assign {
                rvalue: Rvalue::StructInit { name, fields },
                ..
            } = &s.kind
            {
                Some((name.clone(), fields.len()))
            } else {
                None
            }
        });

    let (name, field_count) = init.expect("expected Rvalue::StructInit");
    assert_eq!(name, "Point", "struct init should be for `Point`");
    assert_eq!(field_count, 2, "Point should have 2 fields");
}

#[test]
fn cast_chain() {
    let air = lower_source(
        r#"
fn chain() -> f64 {
    let x: i64 = 42
    let y = x as i32
    let z = y as f64
    return z
}
"#,
    );

    let f = func(&air, "chain");

    let casts: Vec<(AirType, AirType)> = f
        .blocks
        .iter()
        .flat_map(|b| b.stmts.iter())
        .filter_map(|s| {
            if let AirStmtKind::Assign {
                rvalue: Rvalue::Cast { from, to, .. },
                ..
            } = &s.kind
            {
                Some((from.clone(), to.clone()))
            } else {
                None
            }
        })
        .collect();

    assert_eq!(casts.len(), 2, "expected exactly 2 casts, got {}", casts.len());
    assert_eq!(casts[0], (AirType::I64, AirType::I32), "first cast should be I64 → I32");
    assert_eq!(casts[1], (AirType::I32, AirType::F64), "second cast should be I32 → F64");
}

#[test]
fn generic_monomorphized() {
    let air = lower_source(
        r#"
fn identity<T>(x: T) -> T {
    return x
}
fn caller() -> i32 {
    let v: i32 = 42
    return identity(v)
}
"#,
    );

    let mut program = air;
    compute_layouts(&mut program);
    let program = monomorphize(program);

    let mono_fn = program
        .functions
        .iter()
        .find(|f| f.name.contains("__mono_identity_i32"));
    assert!(
        mono_fn.is_some(),
        "expected `__mono_identity_i32` function, found: {:?}",
        program.functions.iter().map(|f| &f.name).collect::<Vec<_>>()
    );

    assert!(
        !program.mono_instances.is_empty(),
        "expected at least one MonoInstance"
    );
    let inst = &program.mono_instances[0];
    assert_eq!(inst.type_args, vec![AirType::I32], "type arg should be I32");

    let caller = func(&program, "caller");
    let has_rewritten_call = caller.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| matches!(&s.kind,
            AirStmtKind::Assign { rvalue: Rvalue::Call { func: Callee::Named(n), .. }, .. }
            if n.contains("__mono_identity_i32")
        ))
        || matches!(&b.terminator,
            AirTerminator::Invoke { func: Callee::Named(n), .. }
            if n.contains("__mono_identity_i32")
        )
    });
    assert!(
        has_rewritten_call,
        "caller's call site should be rewritten to __mono_identity_i32"
    );

    assert!(
        !program.functions.iter().any(|f| f.name == "identity"),
        "original generic `identity` should be removed after monomorphization"
    );
}

#[test]
fn gc_mode_propagation() {
    let air = lower_source(
        r#"
@no_gc
fn manual_func() {
    return
}
fn managed_func() {
    return
}
"#,
    );

    let manual = func(&air, "manual_func");
    let managed = func(&air, "managed_func");

    assert_eq!(
        manual.gc_mode,
        GcMode::Manual,
        "@no_gc function should have Manual gc_mode"
    );
    assert_eq!(
        managed.gc_mode,
        GcMode::Managed,
        "default function should have Managed gc_mode"
    );

    let code = r#"
fn some_func() {
    return
}
"#;
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone()).parse().expect("parse failed");
    let typed = TypeInference::infer_program(stmts, src).expect("sema failed");

    let air_manual = lower_with_gc_mode(&typed, GcMode::Manual);
    let f = func(&air_manual, "some_func");
    assert_eq!(
        f.gc_mode,
        GcMode::Manual,
        "file-level Manual gc mode should propagate to functions"
    );
}

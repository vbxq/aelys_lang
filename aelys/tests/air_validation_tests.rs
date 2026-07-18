use aelys_air::layout::compute_layouts;
use aelys_air::lower::{lower, lower_with_gc_mode};
use aelys_air::mono::monomorphize;
use aelys_air::passes::validate::{AirValidationDetail, validate_air};
use aelys_air::*;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_syntax::Source;
use std::collections::HashSet;

fn lower_source(code: &str) -> AirProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let typed = TypeInference::infer_program(stmts, src).expect("sema failed");
    lower(&typed)
}

fn lower_with_globals(code: &str, globals: &[&str]) -> AirProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let known: HashSet<String> = globals.iter().map(|s| s.to_string()).collect();
    let typed = TypeInference::infer_program_with_imports(stmts, src, HashSet::new(), known)
        .expect("sema failed");
    lower(&typed)
}

fn func<'a>(air: &'a AirProgram, name: &str) -> &'a AirFunction {
    air.functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("function `{}` not found in AIR", name))
}

fn default_attribs() -> FunctionAttribs {
    FunctionAttribs {
        inline: InlineHint::Default,
        no_gc: false,
        no_unwind: false,
        cold: false,
    }
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

    let has_print_call =
        f.blocks.iter().any(|b| {
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

    let has_i32_arg = call_args
        .iter()
        .any(|op| matches!(op, Operand::Const(AirConst::Int(2, AirIntSize::I32))));
    let has_i16_arg = call_args
        .iter()
        .any(|op| matches!(op, Operand::Const(AirConst::Int(2, AirIntSize::I16))));
    assert!(
        has_i32_arg,
        "expected Int(2, I32) argument for take_i32 call"
    );
    assert!(
        has_i16_arg,
        "expected Int(2, I16) argument for take_i16 call"
    );
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
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let result = TypeInference::infer_program(stmts, src);
    assert!(
        result.is_err(),
        "expected sema to reject overflow literal 300 for i8"
    );
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
        "closure env should not capture global `print`, got: {:?}",
        field_names
    );
    assert!(
        !field_names.contains(&"println"),
        "closure env should not capture global `println`, got: {:?}",
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
        b.stmts.iter().any(|s| {
            matches!(&s.kind,
                AirStmtKind::Assign { rvalue: Rvalue::FieldAccess { field, .. }, .. }
                if field == "x"
            )
        })
    });
    assert!(
        has_field_access,
        "expected Rvalue::FieldAccess {{ field: \"x\" }}"
    );

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
        let local = f
            .locals
            .iter()
            .find(|l| l.id == lid)
            .expect("local not found");
        assert_eq!(
            local.ty,
            AirType::I64,
            "field access result local should be I64"
        );
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

    let init = f.blocks.iter().flat_map(|b| b.stmts.iter()).find_map(|s| {
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

    assert_eq!(
        casts.len(),
        2,
        "expected exactly 2 casts, got {}",
        casts.len()
    );
    assert_eq!(
        casts[0],
        (AirType::I64, AirType::I32),
        "first cast should be I64 → I32"
    );
    assert_eq!(
        casts[1],
        (AirType::I32, AirType::F64),
        "second cast should be I32 → F64"
    );
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
    let program = monomorphize(program).unwrap();

    let mono_fn = program
        .functions
        .iter()
        .find(|f| f.name.contains("__mono_identity_i32"));
    assert!(
        mono_fn.is_some(),
        "expected `__mono_identity_i32` function, found: {:?}",
        program
            .functions
            .iter()
            .map(|f| &f.name)
            .collect::<Vec<_>>()
    );

    assert!(
        !program.mono_instances.is_empty(),
        "expected at least one MonoInstance"
    );
    let inst = &program.mono_instances[0];
    assert_eq!(inst.type_args, vec![AirType::I32], "type arg should be I32");

    let caller = func(&program, "caller");
    let has_rewritten_call = caller.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(&s.kind,
                AirStmtKind::Assign { rvalue: Rvalue::Call { func: Callee::Named(n), .. }, .. }
                if n.contains("__mono_identity_i32")
            )
        }) || matches!(&b.terminator,
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

    // verify that monomorphization patched the caller's local type
    // to match the monomorphized return type (was I64 placeholder from Dynamic).
    let call_result_local = caller.blocks.iter().find_map(|b| {
        b.stmts.iter().find_map(|s| match &s.kind {
            AirStmtKind::Assign {
                place: Place::Local(id),
                rvalue:
                    Rvalue::Call {
                        func: Callee::Named(n),
                        ..
                    },
            } if n.contains("__mono_identity_i32") => Some(*id),
            _ => None,
        })
    });
    if let Some(local_id) = call_result_local {
        let local_ty = caller
            .locals
            .iter()
            .find(|l| l.id == local_id)
            .map(|l| &l.ty);
        assert_eq!(
            local_ty,
            Some(&AirType::I32),
            "caller's temp for identity<i32> result should have type I32 after mono, not I64"
        );
    }
}

#[test]
fn generic_struct_decl_only_does_not_introduce_unresolved_air_params() {
    let mut air = lower_source(
        r#"
struct Box<T> { value: T }
fn main() {
}
"#,
    );
    compute_layouts(&mut air);
    let mut air = monomorphize(air).unwrap();
    passes::copy_elim::eliminate_copies(&mut air);
    passes::dead_locals::eliminate_dead_locals(&mut air);

    let result = validate_air(&air);
    assert!(
        result.is_ok(),
        "generic struct declaration without instantiation should not produce unresolved AIR params, errors: {:?}",
        result.err()
    );
    assert!(
        !air.structs.iter().any(|s| s.name == "Box"),
        "uninstantiated generic struct declarations should not be lowered to AIR structs"
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
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let typed = TypeInference::infer_program(stmts, src).expect("sema failed");

    let air_manual = lower_with_gc_mode(&typed, GcMode::Manual);
    let f = func(&air_manual, "some_func");
    assert_eq!(
        f.gc_mode,
        GcMode::Manual,
        "file-level Manual gc mode should propagate to functions"
    );
}

#[test]
fn copy_elim_removes_param_to_local_copies() {
    let mut air = lower_source(
        r#"
fn align_probe(x: i64, y: i32, z: i16, w: i8, b: bool, f: f32, d: f64, p: string) -> i64 {
    let a64: i64 = x
    let a32: i32 = y
    let a16: i16 = z
    let a8: i8 = w
    let ab: bool = b
    let af32: f32 = f
    let af64: f64 = d
    let sp: string = p
    return a64 + (a32 as i64) + (a16 as i64) + (a8 as i64) + (ab as i64) + (af32 as i64) + (af64 as i64)
}
"#,
    );
    compute_layouts(&mut air);
    let mut air = monomorphize(air).unwrap();
    passes::copy_elim::eliminate_copies(&mut air);

    let f = func(&air, "align_probe");
    let params: HashSet<LocalId> = f.params.iter().map(|p| p.id).collect();

    let has_param_copy = f.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(&s.kind,
                AirStmtKind::Assign {
                    place: Place::Local(dst),
                    rvalue: Rvalue::Use(Operand::Copy(src) | Operand::Move(src)),
                }
                if params.contains(src) && !params.contains(dst)
            )
        })
    });
    assert!(
        !has_param_copy,
        "copy elimination should remove param-to-local copies"
    );
}

#[test]
fn copy_elim_keeps_reassigned_param_copy() {
    let mut air = lower_source(
        r#"
fn keep_copy(x: i64) -> i64 {
    let mut y: i64 = x
    y = y + 1
    return y
}
"#,
    );
    compute_layouts(&mut air);
    let mut air = monomorphize(air).unwrap();
    aelys_air::passes::copy_elim::eliminate_copies(&mut air);

    let f = func(&air, "keep_copy");
    let params: HashSet<LocalId> = f.params.iter().map(|p| p.id).collect();

    let has_param_copy = f.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(&s.kind,
                AirStmtKind::Assign {
                    place: Place::Local(dst),
                    rvalue: Rvalue::Use(Operand::Copy(src) | Operand::Move(src)),
                }
                if params.contains(src) && !params.contains(dst)
            )
        })
    });
    assert!(
        has_param_copy,
        "copy should remain when destination local is reassigned"
    );
}

#[test]
fn stdlib_call_println_uses_str_operand() {
    let air = lower_with_globals(
        r#"
fn main() {
    println("Hello")
}
"#,
        &["print", "println"],
    );

    let f = func(&air, "main");
    let arg = f
        .blocks
        .iter()
        .flat_map(|b| b.stmts.iter())
        .find_map(|s| match &s.kind {
            AirStmtKind::CallVoid {
                func: Callee::Named(n),
                args,
            }
            | AirStmtKind::Assign {
                rvalue:
                    Rvalue::Call {
                        func: Callee::Named(n),
                        args,
                    },
                ..
            } if n == "println" => args.first(),
            _ => None,
        })
        .expect("expected a println call");

    match arg {
        Operand::Const(AirConst::Str(_)) => {}
        Operand::Copy(id) | Operand::Move(id) => {
            let ty = f
                .params
                .iter()
                .find(|p| p.id == *id)
                .map(|p| &p.ty)
                .or_else(|| f.locals.iter().find(|l| l.id == *id).map(|l| &l.ty))
                .expect("println argument local should exist");
            assert_eq!(ty, &AirType::Str, "println argument must be `str`");
        }
        _ => panic!("unexpected println arg operand kind"),
    }
}

#[test]
fn dead_locals_removes_align_probe_copy_targets() {
    let mut air = lower_source(
        r#"
fn align_probe(x: i64, y: i32, z: i16, w: i8, b: bool, f: f32, d: f64, p: string) -> i64 {
    let a64: i64 = x
    let a32: i32 = y
    let a16: i16 = z
    let a8: i8 = w
    let ab: bool = b
    let af32: f32 = f
    let af64: f64 = d
    let sp: string = p
    return a64 + (a32 as i64) + (a16 as i64) + (a8 as i64) + (ab as i64) + (af32 as i64) + (af64 as i64)
}
"#,
    );
    compute_layouts(&mut air);
    let mut air = monomorphize(air).unwrap();
    passes::copy_elim::eliminate_copies(&mut air);
    passes::dead_locals::eliminate_dead_locals(&mut air);

    let f = func(&air, "align_probe");
    let local_ids: HashSet<u32> = f.locals.iter().map(|l| l.id.0).collect();
    for id in 8..=14 {
        assert!(
            !local_ids.contains(&id),
            "local %{} should be removed by dead_locals",
            id
        );
    }
}

#[test]
fn multi_instantiation_generic_dispatches_correctly() {
    let air = lower_source(
        r#"
fn identity<T>(x: T) -> T {
    return x
}
fn caller() -> i64 {
    let a: i32 = identity(42 as i32)
    let b: i64 = identity(100)
    return (a as i64) + b
}
"#,
    );

    let mut program = air;
    compute_layouts(&mut program);
    let program = monomorphize(program).unwrap();

    // Both instantiations should exist
    let mono_i32 = program
        .functions
        .iter()
        .find(|f| f.name.contains("__mono_identity_i32"));
    let mono_i64 = program
        .functions
        .iter()
        .find(|f| f.name.contains("__mono_identity_i64"));
    assert!(
        mono_i32.is_some(),
        "expected __mono_identity_i32, found: {:?}",
        program
            .functions
            .iter()
            .map(|f| &f.name)
            .collect::<Vec<_>>()
    );
    assert!(
        mono_i64.is_some(),
        "expected __mono_identity_i64, found: {:?}",
        program
            .functions
            .iter()
            .map(|f| &f.name)
            .collect::<Vec<_>>()
    );

    // verify the i32 instance has i32 param/return, and the i64 instance has i64
    let mono_i32 = mono_i32.unwrap();
    assert_eq!(mono_i32.params[0].ty, AirType::I32);
    assert_eq!(mono_i32.ret_ty, AirType::I32);
    let mono_i64 = mono_i64.unwrap();
    assert_eq!(mono_i64.params[0].ty, AirType::I64);
    assert_eq!(mono_i64.ret_ty, AirType::I64);

    // verify call sites too are rewritten to the correct mangled names
    let caller = func(&program, "caller");
    let call_targets: Vec<String> = caller
        .blocks
        .iter()
        .flat_map(|b| {
            b.stmts
                .iter()
                .filter_map(|s| match &s.kind {
                    AirStmtKind::Assign {
                        rvalue:
                            Rvalue::Call {
                                func: Callee::Named(n),
                                ..
                            },
                        ..
                    } => Some(n.clone()),
                    _ => None,
                })
                .chain(match &b.terminator {
                    AirTerminator::Invoke {
                        func: Callee::Named(n),
                        ..
                    } => Some(n.clone()),
                    _ => None,
                })
        })
        .collect();

    assert!(
        call_targets
            .iter()
            .any(|n| n.contains("__mono_identity_i32")),
        "caller should call __mono_identity_i32, found calls: {:?}",
        call_targets
    );
    assert!(
        call_targets
            .iter()
            .any(|n| n.contains("__mono_identity_i64")),
        "caller should call __mono_identity_i64, found calls: {:?}",
        call_targets
    );
}

// AIR validation pass tests

/// helper: build a minimal valid AirProgram with one function
fn make_valid_program() -> AirProgram {
    AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "test_fn".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![],
            ret_ty: AirType::I64,
            locals: vec![AirLocal {
                id: LocalId(0),
                ty: AirType::I64,
                name: Some("_ret".to_string()),
                is_mut: false,
                span: None,
            }],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![],
                terminator: AirTerminator::Return(Some(Operand::Const(AirConst::Int(
                    0,
                    AirIntSize::I64,
                )))),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: FunctionAttribs {
                inline: InlineHint::Default,
                no_gc: false,
                no_unwind: false,
                cold: false,
            },
            span: None,
        }],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    }
}

#[test]
fn validate_rejects_void_local_non_return() {
    let mut program = make_valid_program();
    // add a Void-typed local that is not the return position.
    program.functions[0].locals.push(AirLocal {
        id: LocalId(1),
        ty: AirType::Void,
        name: Some("bad_local".to_string()),
        is_mut: false,
        span: None,
    });

    let result = validate_air(&program);
    assert!(
        result.is_err(),
        "expected validation to fail for Void local"
    );
    let errors = result.unwrap_err();
    assert_eq!(
        errors.len(),
        1,
        "expected exactly 1 error, got {}",
        errors.len()
    );
    assert!(
        matches!(
            &errors[0].detail,
            AirValidationDetail::VoidLocal { local_id: 1, .. }
        ),
        "expected VoidLocal error for local %1, got: {:?}",
        errors[0].detail
    );
    assert_eq!(errors[0].function_name, "test_fn");
}

#[test]
fn validate_accepts_void_return_local_on_void_function() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "void_fn".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![],
            ret_ty: AirType::Void,
            locals: vec![AirLocal {
                id: LocalId(0),
                ty: AirType::Void,
                name: Some("_ret".to_string()),
                is_mut: false,
                span: None,
            }],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![],
                terminator: AirTerminator::Return(None),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: FunctionAttribs {
                inline: InlineHint::Default,
                no_gc: false,
                no_unwind: false,
                cold: false,
            },
            span: None,
        }],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let result = validate_air(&program);
    assert!(
        result.is_ok(),
        "void return local on void function should be valid"
    );
}

#[test]
fn validate_rejects_void_param() {
    let mut program = make_valid_program();
    program.functions[0].params.push(AirParam {
        id: LocalId(10),
        ty: AirType::Void,
        name: "bad_param".to_string(),
        span: None,
    });

    let result = validate_air(&program);
    assert!(
        result.is_err(),
        "expected validation to fail for Void param"
    );
    let errors = result.unwrap_err();
    assert!(
        errors.iter().any(|e| matches!(
            &e.detail,
            AirValidationDetail::VoidLocal { local_id: 10, .. }
        )),
        "expected VoidLocal error for param %10, got: {:?}",
        errors
    );
}

#[test]
fn function_param_call_lowers_to_indirect_call() {
    let air = lower_source(
        r#"
fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    return f(x)
}
"#,
    );
    let f = func(&air, "apply");
    let has_indirect = f.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(
                &s.kind,
                AirStmtKind::Assign {
                    rvalue: Rvalue::Call {
                        func: Callee::FnPtr(_),
                        ..
                    },
                    ..
                }
            )
        })
    });
    assert!(
        has_indirect,
        "expected call through function parameter to lower as Callee::FnPtr"
    );
}

#[test]
fn struct_fnptr_field_call_lowers_to_indirect_call() {
    let air = lower_source(
        r#"
struct Holder {
    f: fn(i64) -> i64,
}

fn inc(x: i64) -> i64 {
    return x + 1
}

fn main() -> i64 {
    let h = Holder { f: inc }
    return h.f(41)
}
"#,
    );
    let f = func(&air, "main");
    let has_bad_named_call = f.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(
                &s.kind,
                AirStmtKind::Assign {
                    rvalue: Rvalue::Call {
                        func: Callee::Named(name),
                        ..
                    },
                    ..
                } if name == "h.f"
            )
        })
    });
    let has_indirect = f.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(
                &s.kind,
                AirStmtKind::Assign {
                    rvalue: Rvalue::Call {
                        func: Callee::FnPtr(_),
                        ..
                    },
                    ..
                }
            )
        })
    });
    assert!(
        !has_bad_named_call,
        "struct fnptr field call must not lower as direct named call"
    );
    assert!(
        has_indirect,
        "expected struct fnptr field call to lower as Callee::FnPtr"
    );
}

#[test]
fn function_identifier_as_value_lowers_to_fnref() {
    let air = lower_source(
        r#"
fn apply(f: fn(i64) -> i64, x: i64) -> i64 {
    return f(x)
}
fn inc(x: i64) -> i64 {
    return x + 1
}
fn main() -> i64 {
    return apply(inc, 5)
}
"#,
    );
    let f = func(&air, "main");
    let has_closure_create = f.blocks.iter().any(|b| {
        b.stmts.iter().any(|s| {
            matches!(
                &s.kind,
                AirStmtKind::Assign {
                    rvalue: Rvalue::ClosureCreate { fn_name, .. },
                    ..
                } if fn_name == "inc"
            )
        })
    });
    assert!(
        has_closure_create,
        "expected function identifier value to materialize from ClosureCreate(\"inc\")"
    );
}

#[test]
fn validate_rejects_undeclared_block_reference() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "bad_block_ref".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![],
            ret_ty: AirType::Void,
            locals: vec![],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![],
                // References block bb99 which does not exist.
                terminator: AirTerminator::Goto(BlockId(99)),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: FunctionAttribs {
                inline: InlineHint::Default,
                no_gc: false,
                no_unwind: false,
                cold: false,
            },
            span: None,
        }],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let result = validate_air(&program);
    assert!(
        result.is_err(),
        "expected validation to fail for undeclared block"
    );
    let errors = result.unwrap_err();
    assert!(
        errors.iter().any(|e| matches!(
            &e.detail,
            AirValidationDetail::UndeclaredBlock { block_id: 99, .. }
        )),
        "expected UndeclaredBlock error for bb99, got: {:?}",
        errors
    );
}

#[test]
fn validate_rejects_undeclared_local_reference() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "bad_local_ref".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![],
            ret_ty: AirType::I64,
            locals: vec![AirLocal {
                id: LocalId(0),
                ty: AirType::I64,
                name: None,
                is_mut: false,
                span: None,
            }],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![],
                // returns a reference to local %42 which doesn't exist
                terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(42)))),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: FunctionAttribs {
                inline: InlineHint::Default,
                no_gc: false,
                no_unwind: false,
                cold: false,
            },
            span: None,
        }],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let result = validate_air(&program);
    assert!(
        result.is_err(),
        "expected validation to fail for undeclared local"
    );
    let errors = result.unwrap_err();
    assert!(
        errors.iter().any(|e| matches!(
            &e.detail,
            AirValidationDetail::UndeclaredLocal { local_id: 42, .. }
        )),
        "expected UndeclaredLocal error for %42, got: {:?}",
        errors
    );
}

#[test]
fn validate_accepts_valid_lowered_program() {
    // real program lowered from source should pass validation.
    let air = lower_source(
        r#"
fn add(a: i64, b: i64) -> i64 {
    return a + b
}
"#,
    );
    let result = validate_air(&air);
    assert!(
        result.is_ok(),
        "valid lowered program should pass validation, errors: {:?}",
        result.err()
    );
}

#[test]
fn validate_accepts_valid_program_after_full_pipeline() {
    // full pipeline: lower -> layouts -> mono -> copy_elim -> dead_locals -> validate
    let mut air = lower_source(
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
    compute_layouts(&mut air);
    let mut air = monomorphize(air).unwrap();
    passes::copy_elim::eliminate_copies(&mut air);
    passes::dead_locals::eliminate_dead_locals(&mut air);

    let result = validate_air(&air);
    assert!(
        result.is_ok(),
        "fully-pipelined program should pass validation, errors: {:?}",
        result.err()
    );
}

#[test]
fn validate_collects_multiple_errors() {
    // a program with multiple violations should report all of them
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "multi_bad".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![],
            ret_ty: AirType::I64,
            locals: vec![
                AirLocal {
                    id: LocalId(0),
                    ty: AirType::I64,
                    name: None,
                    is_mut: false,
                    span: None,
                },
                AirLocal {
                    id: LocalId(1),
                    ty: AirType::Void,
                    name: Some("void1".to_string()),
                    is_mut: false,
                    span: None,
                },
                AirLocal {
                    id: LocalId(2),
                    ty: AirType::Void,
                    name: Some("void2".to_string()),
                    is_mut: false,
                    span: None,
                },
            ],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![],
                terminator: AirTerminator::Return(Some(Operand::Const(AirConst::Int(
                    0,
                    AirIntSize::I64,
                )))),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: FunctionAttribs {
                inline: InlineHint::Default,
                no_gc: false,
                no_unwind: false,
                cold: false,
            },
            span: None,
        }],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let result = validate_air(&program);
    assert!(result.is_err());
    let errors = result.unwrap_err();
    assert_eq!(
        errors.len(),
        2,
        "expected 2 VoidLocal errors (for %1 and %2), got {}",
        errors.len()
    );
}

#[test]
fn validate_skips_extern_functions() {
    // Extern functions have no body and should not be validated.
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "extern_fn".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![AirParam {
                id: LocalId(0),
                ty: AirType::I64,
                name: "x".to_string(),
                span: None,
            }],
            ret_ty: AirType::Void,
            locals: vec![],
            blocks: vec![], // empty body is OK for extern
            is_extern: true,
            calling_conv: CallingConv::C,
            attributes: FunctionAttribs {
                inline: InlineHint::Default,
                no_gc: false,
                no_unwind: false,
                cold: false,
            },
            span: None,
        }],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let result = validate_air(&program);
    assert!(
        result.is_ok(),
        "extern functions should be skipped, errors: {:?}",
        result.err()
    );
}

// opaque type validation tests

#[test]
fn validate_rejects_opaque_local() {
    let mut program = make_valid_program();
    program.functions[0].locals.push(AirLocal {
        id: LocalId(1),
        ty: AirType::Opaque,
        name: Some("unresolved_dynamic".to_string()),
        is_mut: false,
        span: None,
    });

    let result = validate_air(&program);
    assert!(
        result.is_err(),
        "expected validation to fail for Opaque local"
    );
    let errors = result.unwrap_err();
    assert!(
        errors.iter().any(|e| matches!(
            &e.detail,
            AirValidationDetail::OpaqueType { local_id: 1, .. }
        )),
        "expected OpaqueType error for local %1, got: {:?}",
        errors
    );
}

#[test]
fn validate_rejects_opaque_param() {
    let mut program = make_valid_program();
    program.functions[0].params.push(AirParam {
        id: LocalId(10),
        ty: AirType::Opaque,
        name: "opaque_param".to_string(),
        span: None,
    });

    let result = validate_air(&program);
    assert!(
        result.is_err(),
        "expected validation to fail for Opaque param"
    );
    let errors = result.unwrap_err();
    assert!(
        errors.iter().any(|e| matches!(
            &e.detail,
            AirValidationDetail::OpaqueType { local_id: 10, .. }
        )),
        "expected OpaqueType error for param %10, got: {:?}",
        errors
    );
}

#[test]
fn validate_rejects_opaque_nested_in_array() {
    let mut program = make_valid_program();
    program.functions[0].locals.push(AirLocal {
        id: LocalId(2),
        ty: AirType::Array(Box::new(AirType::Opaque), 5),
        name: Some("opaque_array".to_string()),
        is_mut: false,
        span: None,
    });

    let result = validate_air(&program);
    assert!(
        result.is_err(),
        "expected validation to fail for Opaque nested in Array"
    );
    let errors = result.unwrap_err();
    assert!(
        errors.iter().any(|e| matches!(
            &e.detail,
            AirValidationDetail::OpaqueType { local_id: 2, .. }
        )),
        "expected OpaqueType error for local %2, got: {:?}",
        errors
    );
}

#[test]
fn validate_rejects_opaque_struct_field() {
    let program = AirProgram {
        functions: vec![],
        structs: vec![AirStructDef {
            name: "BadStruct".to_string(),
            type_params: vec![],
            fields: vec![AirStructField {
                name: "unresolved".to_string(),
                ty: AirType::Opaque,
                offset: Some(0),
            }],
            is_closure_env: false,
            span: None,
        }],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let result = validate_air(&program);
    assert!(
        result.is_err(),
        "expected validation to fail for Opaque struct field"
    );
    let errors = result.unwrap_err();
    assert!(
        errors.iter().any(|e| matches!(
            &e.detail,
            AirValidationDetail::OpaqueStructField {
                struct_name,
                field_name,
            } if struct_name == "BadStruct" && field_name == "unresolved"
        )),
        "expected OpaqueStructField error, got: {:?}",
        errors
    );
}

#[test]
fn validate_opaque_does_not_appear_after_monomorphization() {
    // a generic function's return type starts as Dynamic -> Opaque in AIR,
    // but monomorphization should replace it with the concrete type

    // After the full pipeline, validation should pass
    let mut air = lower_source(
        r#"
fn identity<T>(x: T) -> T {
    return x
}
fn caller() -> i64 {
    return identity(42)
}
"#,
    );
    compute_layouts(&mut air);
    let mut air = monomorphize(air).unwrap();
    passes::copy_elim::eliminate_copies(&mut air);
    passes::dead_locals::eliminate_dead_locals(&mut air);

    let result = validate_air(&air);
    assert!(
        result.is_ok(),
        "monomorphized generic call should not have Opaque types, errors: {:?}",
        result.err()
    );
}

#[test]
fn validate_print_builtin_does_not_produce_opaque_local() {
    // print/println returns Dynamic, but AIR lowering should emit CallVoid
    // for their calls instead of creating an Opaque-typed temp local.
    let mut air = lower_with_globals(
        r#"
fn main() {
    println("hello world")
}
"#,
        &["print", "println"],
    );
    compute_layouts(&mut air);
    let mut air = monomorphize(air).unwrap();
    passes::copy_elim::eliminate_copies(&mut air);
    passes::dead_locals::eliminate_dead_locals(&mut air);

    let result = validate_air(&air);
    assert!(
        result.is_ok(),
        "println call should not produce Opaque locals, errors: {:?}",
        result.err()
    );

    // verify no locals have Opaque type.
    let f = func(&air, "main");
    for local in &f.locals {
        assert_ne!(
            local.ty,
            AirType::Opaque,
            "local %{} should not have Opaque type after pipeline",
            local.id.0
        );
    }
}

// Tuple/Range -> Opaque (caught by validation)

/// Simulates what happens when a Tuple type survives to AIR:
///
/// the lowering now produces Opaque instead of Void. if such a local ever reaches the validation pass, it should be rejected with an OpaqueType error
/// here we constructs a synthetic AIR program with an Opaque local representing a Tuple or Range that leaked through sema)
#[test]
fn validate_rejects_opaque_from_tuple_or_range() {
    let program = AirProgram {
        functions: vec![AirFunction {
            id: FunctionId(0),
            name: "tuple_leak".to_string(),
            gc_mode: GcMode::Managed,
            type_params: vec![],
            params: vec![],
            ret_ty: AirType::Void,
            locals: vec![AirLocal {
                id: LocalId(0),
                ty: AirType::Opaque,
                name: Some("leaked_tuple".to_string()),
                is_mut: false,
                span: None,
            }],
            blocks: vec![AirBlock {
                id: BlockId(0),
                stmts: vec![],
                terminator: AirTerminator::Return(None),
            }],
            is_extern: false,
            calling_conv: CallingConv::Aelys,
            attributes: FunctionAttribs {
                inline: InlineHint::Default,
                no_gc: false,
                no_unwind: false,
                cold: false,
            },
            span: None,
        }],
        structs: vec![],
        enums: vec![],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let result = validate_air(&program);
    assert!(
        result.is_err(),
        "Opaque local (from Tuple/Range) should be rejected by validation"
    );
    let errors = result.unwrap_err();
    assert!(
        errors.iter().any(|e| matches!(
            &e.detail,
            AirValidationDetail::OpaqueType {
                local_id: 0,
                local_name: Some(name),
            } if name == "leaked_tuple"
        )),
        "expected OpaqueType error for leaked_tuple, got: {:?}",
        errors
    );
}

/// verifies that null-typed locals pass validation (they are Ptr(Void), not bare Void, so they have a valid non-zero size)
#[test]
fn validate_accepts_null_typed_local() {
    let air = lower_source(
        r#"
fn use_null() {
    let x = null
}
"#,
    );
    let result = validate_air(&air);
    assert!(
        result.is_ok(),
        "null-typed local (Ptr(Void)) should pass validation, errors: {:?}",
        result.err()
    );
}

#[test]
fn validate_rejects_ambiguous_generic_unit_variant_after_mono() {
    let option_enum = AirEnumDef {
        name: "Option".to_string(),
        type_params: vec![TypeParamId(0)],
        variants: vec![
            AirEnumVariant {
                name: "Some".to_string(),
                tag: 0,
                payload: vec![AirType::Param(TypeParamId(0))],
            },
            AirEnumVariant {
                name: "None".to_string(),
                tag: 1,
                payload: vec![],
            },
        ],
        span: None,
    };

    let seed_i64 = AirFunction {
        id: FunctionId(0),
        name: "seed_i64".to_string(),
        gc_mode: GcMode::Managed,
        type_params: vec![],
        params: vec![],
        ret_ty: AirType::Enum("__mono_Option_i64".to_string()),
        locals: vec![
            AirLocal {
                id: LocalId(0),
                ty: AirType::Enum("__mono_Option_i64".to_string()),
                name: Some("ret".to_string()),
                is_mut: false,
                span: None,
            },
            AirLocal {
                id: LocalId(1),
                ty: AirType::Enum("__mono_Option_i64".to_string()),
                name: Some("value".to_string()),
                is_mut: false,
                span: None,
            },
        ],
        blocks: vec![AirBlock {
            id: BlockId(0),
            stmts: vec![AirStmt {
                kind: AirStmtKind::Assign {
                    place: Place::Local(LocalId(1)),
                    rvalue: Rvalue::EnumInit {
                        enum_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        tag: 0,
                        payload: vec![Operand::Const(AirConst::Int(1, AirIntSize::I64))],
                    },
                },
                span: None,
            }],
            terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(1)))),
        }],
        is_extern: false,
        calling_conv: CallingConv::Aelys,
        attributes: default_attribs(),
        span: None,
    };

    let seed_str = AirFunction {
        id: FunctionId(1),
        name: "seed_str".to_string(),
        gc_mode: GcMode::Managed,
        type_params: vec![],
        params: vec![],
        ret_ty: AirType::Enum("__mono_Option_str".to_string()),
        locals: vec![
            AirLocal {
                id: LocalId(0),
                ty: AirType::Enum("__mono_Option_str".to_string()),
                name: Some("ret".to_string()),
                is_mut: false,
                span: None,
            },
            AirLocal {
                id: LocalId(1),
                ty: AirType::Enum("__mono_Option_str".to_string()),
                name: Some("value".to_string()),
                is_mut: false,
                span: None,
            },
        ],
        blocks: vec![AirBlock {
            id: BlockId(0),
            stmts: vec![AirStmt {
                kind: AirStmtKind::Assign {
                    place: Place::Local(LocalId(1)),
                    rvalue: Rvalue::EnumInit {
                        enum_name: "Option".to_string(),
                        variant: "Some".to_string(),
                        tag: 0,
                        payload: vec![Operand::Const(AirConst::Str("hello".to_string()))],
                    },
                },
                span: None,
            }],
            terminator: AirTerminator::Return(Some(Operand::Copy(LocalId(1)))),
        }],
        is_extern: false,
        calling_conv: CallingConv::Aelys,
        attributes: default_attribs(),
        span: None,
    };

    let ambiguous_none = AirFunction {
        id: FunctionId(2),
        name: "ambiguous_none".to_string(),
        gc_mode: GcMode::Managed,
        type_params: vec![],
        params: vec![],
        ret_ty: AirType::Void,
        locals: vec![AirLocal {
            id: LocalId(0),
            ty: AirType::Enum("Option".to_string()),
            name: Some("ambiguous".to_string()),
            is_mut: false,
            span: None,
        }],
        blocks: vec![AirBlock {
            id: BlockId(0),
            stmts: vec![AirStmt {
                kind: AirStmtKind::Assign {
                    place: Place::Local(LocalId(0)),
                    rvalue: Rvalue::EnumInit {
                        enum_name: "Option".to_string(),
                        variant: "None".to_string(),
                        tag: 1,
                        payload: vec![],
                    },
                },
                span: None,
            }],
            terminator: AirTerminator::Return(None),
        }],
        is_extern: false,
        calling_conv: CallingConv::Aelys,
        attributes: default_attribs(),
        span: None,
    };

    let program = AirProgram {
        functions: vec![seed_i64, seed_str, ambiguous_none],
        structs: vec![],
        enums: vec![option_enum],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let errors = match monomorphize(program) {
        Err(e) => e,
        Ok(_) => panic!("ambiguous generic unit variant should be rejected during monomorphization"),
    };
    assert!(
        errors.iter().any(|e| e.contains("ambiguous unit variant")),
        "expected ambiguous unit variant error, got: {:?}",
        errors
    );
}

#[test]
fn monomorphize_distinguishes_fnptr_calling_conventions_in_enum_type_args() {
    let program = AirProgram {
        functions: vec![
            AirFunction {
                id: FunctionId(0),
                name: "fast_fn".to_string(),
                gc_mode: GcMode::Managed,
                type_params: vec![],
                params: vec![],
                ret_ty: AirType::I64,
                locals: vec![],
                blocks: vec![],
                is_extern: true,
                calling_conv: CallingConv::Aelys,
                attributes: default_attribs(),
                span: None,
            },
            AirFunction {
                id: FunctionId(1),
                name: "c_fn".to_string(),
                gc_mode: GcMode::Managed,
                type_params: vec![],
                params: vec![],
                ret_ty: AirType::I64,
                locals: vec![],
                blocks: vec![],
                is_extern: true,
                calling_conv: CallingConv::C,
                attributes: default_attribs(),
                span: None,
            },
            AirFunction {
                id: FunctionId(2),
                name: "seed".to_string(),
                gc_mode: GcMode::Managed,
                type_params: vec![],
                params: vec![],
                ret_ty: AirType::Void,
                locals: vec![
                    AirLocal {
                        id: LocalId(0),
                        ty: AirType::FnPtr {
                            params: vec![],
                            ret: Box::new(AirType::I64),
                            conv: CallingConv::Aelys,
                        },
                        name: Some("fast".to_string()),
                        is_mut: false,
                        span: None,
                    },
                    AirLocal {
                        id: LocalId(1),
                        ty: AirType::FnPtr {
                            params: vec![],
                            ret: Box::new(AirType::I64),
                            conv: CallingConv::C,
                        },
                        name: Some("c".to_string()),
                        is_mut: false,
                        span: None,
                    },
                    AirLocal {
                        id: LocalId(2),
                        ty: AirType::Enum("Holder".to_string()),
                        name: Some("aelys_holder".to_string()),
                        is_mut: false,
                        span: None,
                    },
                    AirLocal {
                        id: LocalId(3),
                        ty: AirType::Enum("Holder".to_string()),
                        name: Some("c_holder".to_string()),
                        is_mut: false,
                        span: None,
                    },
                ],
                blocks: vec![AirBlock {
                    id: BlockId(0),
                    stmts: vec![
                        AirStmt {
                            kind: AirStmtKind::Assign {
                                place: Place::Local(LocalId(0)),
                                rvalue: Rvalue::Use(Operand::Const(AirConst::FnRef(
                                    "fast_fn".to_string(),
                                ))),
                            },
                            span: None,
                        },
                        AirStmt {
                            kind: AirStmtKind::Assign {
                                place: Place::Local(LocalId(1)),
                                rvalue: Rvalue::Use(Operand::Const(AirConst::FnRef(
                                    "c_fn".to_string(),
                                ))),
                            },
                            span: None,
                        },
                        AirStmt {
                            kind: AirStmtKind::Assign {
                                place: Place::Local(LocalId(2)),
                                rvalue: Rvalue::EnumInit {
                                    enum_name: "Holder".to_string(),
                                    variant: "Value".to_string(),
                                    tag: 0,
                                    payload: vec![Operand::Copy(LocalId(0))],
                                },
                            },
                            span: None,
                        },
                        AirStmt {
                            kind: AirStmtKind::Assign {
                                place: Place::Local(LocalId(3)),
                                rvalue: Rvalue::EnumInit {
                                    enum_name: "Holder".to_string(),
                                    variant: "Value".to_string(),
                                    tag: 0,
                                    payload: vec![Operand::Copy(LocalId(1))],
                                },
                            },
                            span: None,
                        },
                    ],
                    terminator: AirTerminator::Return(None),
                }],
                is_extern: false,
                calling_conv: CallingConv::Aelys,
                attributes: default_attribs(),
                span: None,
            },
        ],
        structs: vec![],
        enums: vec![AirEnumDef {
            name: "Holder".to_string(),
            type_params: vec![TypeParamId(0)],
            variants: vec![
                AirEnumVariant {
                    name: "Value".to_string(),
                    tag: 0,
                    payload: vec![AirType::Param(TypeParamId(0))],
                },
                AirEnumVariant {
                    name: "Empty".to_string(),
                    tag: 1,
                    payload: vec![],
                },
            ],
            span: None,
        }],
        globals: vec![],
        source_files: vec![],
        mono_instances: vec![],
        struct_sizes: std::collections::HashMap::new(),
        rc_type_table: aelys_air::rc_types::RcTypeTable::default(),
    };

    let air = monomorphize(program).unwrap();
    let holder_defs: Vec<_> = air
        .enums
        .iter()
        .filter(|def| def.name.starts_with("__mono_Holder_"))
        .collect();
    assert_eq!(
        holder_defs.len(),
        2,
        "distinct fnptr calling conventions must produce distinct Holder monos"
    );
    assert!(
        holder_defs.iter().any(|def| matches!(
            &def.variants[0].payload[..],
            [AirType::FnPtr { conv: CallingConv::Aelys, .. }]
        )),
        "missing Aelys fnptr instantiation: {:?}",
        holder_defs.iter().map(|def| &def.name).collect::<Vec<_>>()
    );
    assert!(
        holder_defs.iter().any(|def| matches!(
            &def.variants[0].payload[..],
            [AirType::FnPtr { conv: CallingConv::C, .. }]
        )),
        "missing C fnptr instantiation: {:?}",
        holder_defs.iter().map(|def| &def.name).collect::<Vec<_>>()
    );
}

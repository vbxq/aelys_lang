use aelys::run;
use aelys_runtime::Value;
use aelys_sema::types::{InferType, ResolvedType, TypeVarId};
use aelys_sema::{StructDef, StructField, Substitution, TypeTable};

fn run_ok(source: &str) -> Value {
    run(source, "test.aelys").expect("program should succeed")
}

#[allow(dead_code)]
fn run_err(source: &str) -> String {
    run(source, "test.aelys")
        .expect_err("program should fail")
        .to_string()
}

fn parse(source: &str) -> Vec<aelys_syntax::Stmt> {
    let src = aelys_syntax::Source::new("<test>", source);
    let tokens = aelys_frontend::lexer::Lexer::with_source(src.clone())
        .scan()
        .unwrap();
    aelys_frontend::parser::Parser::new(tokens, src)
        .parse()
        .unwrap()
}

fn infer(source: &str) -> aelys_sema::InferenceResult {
    let src = aelys_syntax::Source::new("<test>", source);
    let tokens = aelys_frontend::lexer::Lexer::with_source(src.clone())
        .scan()
        .unwrap();
    let ast = aelys_frontend::parser::Parser::new(tokens, src.clone())
        .parse()
        .unwrap();
    aelys_sema::TypeInference::infer_program_full(ast, src, Default::default(), Default::default())
        .unwrap()
}

fn make_ann(name: &str) -> aelys_syntax::TypeAnnotation {
    aelys_syntax::TypeAnnotation::new(name.to_string(), aelys_syntax::Span::new(0, 0, 1, 1))
}

// ---------------------------------------------------------------------------
// InferType sized variants
// ---------------------------------------------------------------------------

#[test]
fn infer_type_from_annotation_sized_integers() {
    assert_eq!(InferType::from_annotation(&make_ann("int")), InferType::I64);
    assert_eq!(InferType::from_annotation(&make_ann("i64")), InferType::I64);
    assert_eq!(InferType::from_annotation(&make_ann("i32")), InferType::I32);
    assert_eq!(InferType::from_annotation(&make_ann("i16")), InferType::I16);
    assert_eq!(InferType::from_annotation(&make_ann("i8")), InferType::I8);
    assert_eq!(InferType::from_annotation(&make_ann("u64")), InferType::U64);
    assert_eq!(InferType::from_annotation(&make_ann("u32")), InferType::U32);
    assert_eq!(InferType::from_annotation(&make_ann("u16")), InferType::U16);
    assert_eq!(InferType::from_annotation(&make_ann("u8")), InferType::U8);
}

#[test]
fn infer_type_from_annotation_sized_floats() {
    assert_eq!(
        InferType::from_annotation(&make_ann("float")),
        InferType::F64
    );
    assert_eq!(InferType::from_annotation(&make_ann("f64")), InferType::F64);
    assert_eq!(InferType::from_annotation(&make_ann("f32")), InferType::F32);
}

#[test]
fn infer_type_from_annotation_struct() {
    assert_eq!(
        InferType::from_annotation(&make_ann("Point")),
        InferType::Struct("Point".to_string())
    );
    assert_eq!(
        InferType::from_annotation(&make_ann("MyStruct")),
        InferType::Struct("MyStruct".to_string())
    );
}

#[test]
fn infer_type_from_annotation_case_insensitive_builtins() {
    assert_eq!(InferType::from_annotation(&make_ann("Int")), InferType::I64);
    assert_eq!(
        InferType::from_annotation(&make_ann("Float")),
        InferType::F64
    );
    assert_eq!(InferType::from_annotation(&make_ann("I32")), InferType::I32);
}

#[test]
fn infer_type_is_integer() {
    assert!(InferType::I8.is_integer());
    assert!(InferType::I16.is_integer());
    assert!(InferType::I32.is_integer());
    assert!(InferType::I64.is_integer());
    assert!(InferType::U8.is_integer());
    assert!(InferType::U16.is_integer());
    assert!(InferType::U32.is_integer());
    assert!(InferType::U64.is_integer());
    assert!(!InferType::F32.is_integer());
    assert!(!InferType::F64.is_integer());
    assert!(!InferType::Bool.is_integer());
    assert!(!InferType::String.is_integer());
}

#[test]
fn infer_type_is_float() {
    assert!(InferType::F32.is_float());
    assert!(InferType::F64.is_float());
    assert!(!InferType::I64.is_float());
    assert!(!InferType::Bool.is_float());
}

#[test]
fn infer_type_is_numeric() {
    assert!(InferType::I64.is_numeric());
    assert!(InferType::F32.is_numeric());
    assert!(!InferType::Bool.is_numeric());
    assert!(!InferType::Struct("X".to_string()).is_numeric());
}

#[test]
fn infer_type_sized_variants_are_concrete() {
    assert!(InferType::I8.is_concrete());
    assert!(InferType::U64.is_concrete());
    assert!(InferType::F32.is_concrete());
    assert!(InferType::Struct("Foo".to_string()).is_concrete());
    assert!(!InferType::Var(TypeVarId(0)).is_concrete());
    assert!(!InferType::Dynamic.is_concrete());
}

#[test]
fn infer_type_sized_variants_no_vars() {
    assert!(!InferType::I32.has_vars());
    assert!(!InferType::U8.has_vars());
    assert!(!InferType::F64.has_vars());
    assert!(!InferType::Struct("Vec3".to_string()).has_vars());
}

#[test]
fn infer_type_display_sized() {
    assert_eq!(format!("{}", InferType::I8), "i8");
    assert_eq!(format!("{}", InferType::I64), "i64");
    assert_eq!(format!("{}", InferType::U32), "u32");
    assert_eq!(format!("{}", InferType::F32), "f32");
    assert_eq!(format!("{}", InferType::F64), "f64");
    assert_eq!(
        format!("{}", InferType::Struct("Point".to_string())),
        "Point"
    );
}

// ---------------------------------------------------------------------------
// ResolvedType sized variants
// ---------------------------------------------------------------------------

#[test]
fn resolved_type_is_integer() {
    assert!(ResolvedType::I64.is_integer());
    assert!(ResolvedType::U8.is_integer());
    assert!(!ResolvedType::F64.is_integer());
}

#[test]
fn resolved_type_is_float() {
    assert!(ResolvedType::F64.is_float());
    assert!(ResolvedType::F32.is_float());
    assert!(!ResolvedType::I64.is_float());
}

#[test]
fn resolved_type_is_integer_ish() {
    assert!(ResolvedType::I64.is_integer_ish());
    assert!(ResolvedType::U32.is_integer_ish());
    assert!(ResolvedType::Uncertain(Box::new(ResolvedType::I64)).is_integer_ish());
    assert!(!ResolvedType::F64.is_integer_ish());
    assert!(!ResolvedType::Dynamic.is_integer_ish());
}

#[test]
fn resolved_type_from_infer_type_sized() {
    assert_eq!(
        ResolvedType::from_infer_type(&InferType::I8),
        ResolvedType::I8
    );
    assert_eq!(
        ResolvedType::from_infer_type(&InferType::U64),
        ResolvedType::U64
    );
    assert_eq!(
        ResolvedType::from_infer_type(&InferType::F32),
        ResolvedType::F32
    );
    assert_eq!(
        ResolvedType::from_infer_type(&InferType::Struct("P".to_string())),
        ResolvedType::Struct("P".to_string())
    );
}

// ---------------------------------------------------------------------------
// TypeTable
// ---------------------------------------------------------------------------

#[test]
fn type_table_register_and_get() {
    let mut table = TypeTable::new();
    table.register_struct(StructDef {
        name: "Point".to_string(),
        type_params: Vec::new(),
        fields: vec![
            StructField {
                name: "x".to_string(),
                ty: InferType::F64,
            },
            StructField {
                name: "y".to_string(),
                ty: InferType::F64,
            },
        ],
    });

    assert!(table.has_struct("Point"));
    assert!(!table.has_struct("Line"));

    let def = table.get_struct("Point").unwrap();
    assert_eq!(def.fields.len(), 2);
    assert_eq!(def.fields[0].name, "x");
    assert_eq!(def.fields[1].ty, InferType::F64);
}

// ---------------------------------------------------------------------------
// Parser: struct declarations
// ---------------------------------------------------------------------------

#[test]
fn parse_struct_declaration() {
    let stmts = parse("struct Point { x: f64, y: f64 }");
    assert_eq!(stmts.len(), 1);

    match &stmts[0].kind {
        aelys_syntax::StmtKind::StructDecl {
            name,
            type_params,
            fields,
            is_pub,
        } => {
            assert_eq!(name, "Point");
            assert!(type_params.is_empty());
            assert!(!is_pub);
            assert_eq!(fields.len(), 2);
            assert_eq!(fields[0].name, "x");
            assert_eq!(fields[1].name, "y");
        }
        _ => panic!("expected StructDecl"),
    }
}

#[test]
fn parse_pub_struct_declaration() {
    let stmts = parse("pub struct Color { r: u8, g: u8, b: u8 }");
    assert_eq!(stmts.len(), 1);

    match &stmts[0].kind {
        aelys_syntax::StmtKind::StructDecl {
            name,
            is_pub,
            fields,
            ..
        } => {
            assert_eq!(name, "Color");
            assert!(is_pub);
            assert_eq!(fields.len(), 3);
        }
        _ => panic!("expected StructDecl"),
    }
}

#[test]
fn parse_struct_trailing_comma() {
    let stmts = parse("struct Pair { a: int, b: int, }");
    match &stmts[0].kind {
        aelys_syntax::StmtKind::StructDecl { fields, .. } => {
            assert_eq!(fields.len(), 2);
        }
        _ => panic!("expected StructDecl"),
    }
}

// ---------------------------------------------------------------------------
// Parser: struct literals
// ---------------------------------------------------------------------------

#[test]
fn parse_struct_literal() {
    let stmts = parse("Point { x: 1, y: 2 }");
    assert_eq!(stmts.len(), 1);

    match &stmts[0].kind {
        aelys_syntax::StmtKind::Expression(expr) => match &expr.kind {
            aelys_syntax::ExprKind::StructLiteral { name, fields } => {
                assert_eq!(name, "Point");
                assert_eq!(fields.len(), 2);
                assert_eq!(fields[0].name, "x");
                assert_eq!(fields[1].name, "y");
            }
            _ => panic!("expected StructLiteral, got {:?}", expr.kind),
        },
        _ => panic!("expected expression statement"),
    }
}

#[test]
fn parse_uppercase_identifier_without_brace_is_not_struct_literal() {
    let stmts = parse("let DEBUG = true");
    match &stmts[0].kind {
        aelys_syntax::StmtKind::Let { initializer, .. } => {
            assert!(matches!(
                initializer.kind,
                aelys_syntax::ExprKind::Bool(true)
            ));
        }
        _ => panic!("expected let statement"),
    }
}

#[test]
fn parse_uppercase_var_before_block_is_not_struct_literal() {
    let stmts = parse("if TRUE { 1 } else { 0 }");
    assert!(matches!(stmts[0].kind, aelys_syntax::StmtKind::If { .. }));
}

// ---------------------------------------------------------------------------
// Lexer: struct keyword
// ---------------------------------------------------------------------------

#[test]
fn lexer_recognizes_struct_keyword() {
    let src = aelys_syntax::Source::new("<test>", "struct");
    let tokens = aelys_frontend::lexer::Lexer::with_source(src)
        .scan()
        .unwrap();
    assert!(matches!(tokens[0].kind, aelys_syntax::TokenKind::Struct));
}

// ---------------------------------------------------------------------------
// Sema: type inference with sized types
// ---------------------------------------------------------------------------

#[test]
fn infer_int_literal_as_i64() {
    let result = infer("let x = 42");
    let stmt = &result.program.stmts[0];
    match &stmt.kind {
        aelys_sema::TypedStmtKind::Let { var_type, .. } => {
            let resolved = ResolvedType::from_infer_type(var_type);
            assert!(resolved.is_integer());
        }
        _ => panic!("expected Let"),
    }
}

#[test]
fn infer_float_literal_as_f64() {
    let result = infer("let x = 3.14");
    let stmt = &result.program.stmts[0];
    match &stmt.kind {
        aelys_sema::TypedStmtKind::Let { var_type, .. } => {
            let resolved = ResolvedType::from_infer_type(var_type);
            assert!(resolved.is_float());
        }
        _ => panic!("expected Let"),
    }
}

#[test]
fn infer_struct_declaration_populates_type_table() {
    let result = infer("struct Vec2 { x: f64, y: f64 }");
    assert!(result.type_table.has_struct("Vec2"));
    let def = result.type_table.get_struct("Vec2").unwrap();
    assert_eq!(def.fields.len(), 2);
    assert_eq!(def.fields[0].ty, InferType::F64);
}

#[test]
fn infer_struct_literal_type() {
    let result = infer(
        r#"
        struct Point { x: f64, y: f64 }
        let p = Point { x: 1.0, y: 2.0 }
    "#,
    );
    let let_stmt = &result.program.stmts[1];
    match &let_stmt.kind {
        aelys_sema::TypedStmtKind::Let { var_type, .. } => {
            assert_eq!(*var_type, InferType::Struct("Point".to_string()));
        }
        _ => panic!("expected Let"),
    }
}

#[test]
fn infer_struct_field_access_type() {
    let result = infer(
        r#"
        struct Pair { a: int, b: int }
        let p = Pair { a: 10, b: 20 }
        p.a
    "#,
    );
    let expr_stmt = &result.program.stmts[2];
    match &expr_stmt.kind {
        aelys_sema::TypedStmtKind::Expression(expr) => {
            assert!(matches!(
                &expr.kind,
                aelys_sema::TypedExprKind::Member { .. }
            ));
        }
        _ => panic!("expected Expression"),
    }
}

// ---------------------------------------------------------------------------
// Sema: unification with sized types
// ---------------------------------------------------------------------------

#[test]
fn unify_same_sized_types() {
    let mut subst = Substitution::new();
    assert!(aelys_sema::unify::unify(&InferType::I32, &InferType::I32, &mut subst).is_ok());
    assert!(aelys_sema::unify::unify(&InferType::U64, &InferType::U64, &mut subst).is_ok());
    assert!(aelys_sema::unify::unify(&InferType::F32, &InferType::F32, &mut subst).is_ok());
}

#[test]
fn unify_different_sized_types_fails() {
    let mut subst = Substitution::new();
    assert!(aelys_sema::unify::unify(&InferType::I32, &InferType::I64, &mut subst).is_err());
    assert!(aelys_sema::unify::unify(&InferType::F32, &InferType::F64, &mut subst).is_err());
    assert!(aelys_sema::unify::unify(&InferType::I64, &InferType::F64, &mut subst).is_err());
}

#[test]
fn unify_struct_nominal_same_name() {
    let mut subst = Substitution::new();
    let a = InferType::Struct("Point".to_string());
    let b = InferType::Struct("Point".to_string());
    assert!(aelys_sema::unify::unify(&a, &b, &mut subst).is_ok());
}

#[test]
fn unify_struct_nominal_different_name_fails() {
    let mut subst = Substitution::new();
    let a = InferType::Struct("Point".to_string());
    let b = InferType::Struct("Color".to_string());
    assert!(aelys_sema::unify::unify(&a, &b, &mut subst).is_err());
}

// ---------------------------------------------------------------------------
// E2E: sized type annotations work through the VM
// ---------------------------------------------------------------------------

#[test]
fn e2e_i64_annotation() {
    assert_eq!(run_ok("let x: i64 = 42\nx").as_int(), Some(42));
}

#[test]
fn e2e_int_alias_still_works() {
    assert_eq!(run_ok("let x: int = 99\nx").as_int(), Some(99));
}

#[test]
fn e2e_f64_annotation() {
    let v = run_ok("let x: f64 = 2.718\nx");
    assert!((v.as_float().unwrap() - std::f64::consts::E).abs() < 0.001);
}

#[test]
fn e2e_float_alias_still_works() {
    let v = run_ok("let x: float = 1.5\nx");
    assert!((v.as_float().unwrap() - 1.5).abs() < 0.001);
}

#[test]
fn e2e_i64_arithmetic() {
    let v = run_ok("let a: i64 = 10\nlet b: i64 = 20\na + b");
    assert_eq!(v.as_int(), Some(30));
}

#[test]
fn e2e_f64_arithmetic() {
    let v = run_ok("let a: f64 = 1.5\nlet b: f64 = 2.5\na * b");
    assert!((v.as_float().unwrap() - 3.75).abs() < 0.001);
}

#[test]
fn e2e_struct_declaration_is_noop_in_vm() {
    let v = run_ok("struct Foo { x: int }\n42");
    assert_eq!(v.as_int(), Some(42));
}

#[test]
fn e2e_typed_for_loop() {
    let v = run_ok(
        r#"
        let mut sum: i64 = 0
        for i in 1..=10 {
            sum += i
        }
        sum
    "#,
    );
    assert_eq!(v.as_int(), Some(55));
}

#[test]
fn e2e_typed_function_params() {
    let v = run_ok(
        r#"
        fn add(a: i64, b: i64) -> i64 {
            return a + b
        }
        add(100, 200)
    "#,
    );
    assert_eq!(v.as_int(), Some(300));
}

#[test]
fn e2e_typed_lambda() {
    let v = run_ok(
        r#"
        let mul = fn(a: i64, b: i64) -> i64 { return a * b }
        mul(7, 8)
    "#,
    );
    assert_eq!(v.as_int(), Some(56));
}

#[test]
fn e2e_uppercase_variable_not_confused_with_struct() {
    let v = run_ok(
        r#"
        let MAX = 100
        let result = if MAX > 50 { 1 } else { 0 }
        result
    "#,
    );
    assert_eq!(v.as_int(), Some(1));
}

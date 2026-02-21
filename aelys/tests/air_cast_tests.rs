use aelys_air::lower::lower;
use aelys_air::{AirProgram, AirStmtKind, AirType, Rvalue};
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_syntax::Source;

fn source_to_air(code: &str) -> AirProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone())
        .scan()
        .expect("lexer failed");
    let ast = Parser::new(tokens, src.clone())
        .parse()
        .expect("parser failed");
    let typed = TypeInference::infer_program(ast, src).expect("sema failed");
    lower(&typed)
}

/// Wrap an expression in a function so AIR lowering processes it.
fn wrap(expr: &str) -> String {
    format!("fn test() {{ {} }}", expr)
}

/// Walk all statements in all blocks of all functions, collect Cast rvalues as (from, to) pairs.
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
fn int_to_float_produces_cast() {
    let air = source_to_air(&wrap("42 as f64"));
    let casts = collect_casts(&air);
    assert!(!casts.is_empty(), "expected at least one Cast in AIR");
    assert!(
        casts.iter().any(|(_, to)| *to == &AirType::F64),
        "expected cast target F64, got: {casts:?}"
    );
}

#[test]
fn float_to_int_produces_cast() {
    let air = source_to_air(&wrap("3.14 as i32"));
    let casts = collect_casts(&air);
    assert!(!casts.is_empty(), "expected at least one Cast in AIR");
    assert!(
        casts.iter().any(|(_, to)| *to == &AirType::I32),
        "expected cast target I32, got: {casts:?}"
    );
}

#[test]
fn bool_to_int_produces_cast() {
    let air = source_to_air(&wrap("true as i32"));
    let casts = collect_casts(&air);
    assert!(!casts.is_empty(), "expected at least one Cast in AIR");
    let (from, to) = &casts[0];
    assert_eq!(*from, &AirType::Bool);
    assert_eq!(*to, &AirType::I32);
}

#[test]
fn int_to_bool_produces_cast() {
    let air = source_to_air(&wrap("1 as bool"));
    let casts = collect_casts(&air);
    assert!(!casts.is_empty(), "expected at least one Cast in AIR");
    assert!(
        casts.iter().any(|(_, to)| *to == &AirType::Bool),
        "expected cast target Bool, got: {casts:?}"
    );
}

#[test]
fn chained_cast_produces_two_casts() {
    let air = source_to_air(&wrap("42 as i32 as f64"));
    let casts = collect_casts(&air);
    assert!(
        casts.len() >= 2,
        "expected at least 2 casts for chained cast, got {}",
        casts.len()
    );
}

#[test]
fn no_cast_without_as() {
    let air = source_to_air(&wrap("42 + 1"));
    let casts = collect_casts(&air);
    assert!(casts.is_empty(), "expected no casts, got: {casts:?}");
}

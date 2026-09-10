/// Regression tests for OneOf constraint improvements.
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::types::InferType;
use aelys_sema::{TypeInference, TypedStmtKind};
use aelys_syntax::Source;

fn infer_ok(code: &str) -> aelys_sema::TypedProgram {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    match TypeInference::infer_program(stmts, src) {
        Ok(p) => p,
        Err(errors) => {
            for e in &errors {
                eprintln!("  ERROR: {}", e);
            }
            panic!("expected OK, got {} errors", errors.len());
        }
    }
}

fn sema_ok(code: &str) -> bool {
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    TypeInference::infer_program(stmts, src).is_ok()
}

fn sema_err(code: &str) -> bool {
    !sema_ok(code)
}

/// an untyped parameter used with a binary `+` should default to I64, not Dynamic, because the OneOf constraint says it must be numeric
#[test]
fn oneof_unresolved_var_defaults_to_i64_for_add() {
    let program = infer_ok(
        r#"
fn double(x) {
    return x + x
}
"#,
    );
    // find the function and check its parameter type
    for stmt in &program.stmts {
        if let TypedStmtKind::Function(func) = &stmt.kind {
            if func.name == "double" {
                assert_eq!(
                    func.params[0].ty,
                    InferType::I64,
                    "unresolved param constrained by OneOf(numerics) should default to I64, got {:?}",
                    func.params[0].ty
                );
                return;
            }
        }
    }
    panic!("did not find function 'double' in typed AST");
}

/// an untyped parameter used with `*` should also default to I64.
#[test]
fn oneof_unresolved_var_defaults_to_i64_for_mul() {
    let program = infer_ok(
        r#"
fn square(n) {
    return n * n
}
"#,
    );
    for stmt in &program.stmts {
        if let TypedStmtKind::Function(func) = &stmt.kind {
            if func.name == "square" {
                assert_eq!(
                    func.params[0].ty,
                    InferType::I64,
                    "unresolved param constrained by OneOf(numerics) via * should default to I64, got {:?}",
                    func.params[0].ty
                );
                return;
            }
        }
    }
    panic!("did not find function 'square' in typed AST");
}

/// when a Var is constrained by Equal to a concrete type and by OneOf, the Equal binding should take precedence (OneOf just validates)
#[test]
fn oneof_does_not_override_equal_binding() {
    let program = infer_ok(
        r#"
fn add_i32(a: i32, b: i32) -> i32 {
    return a + b
}
"#,
    );
    for stmt in &program.stmts {
        if let TypedStmtKind::Function(func) = &stmt.kind {
            if func.name == "add_i32" {
                assert_eq!(
                    func.return_type,
                    InferType::I32,
                    "return type should stay I32 from Equal constraint, got {:?}",
                    func.return_type
                );
                return;
            }
        }
    }
    panic!("did not find function 'add_i32' in typed AST");
}

/// an untyped param used with bitwise and should default to I64
/// (since all_integer_types is the option set for bitwise ops)
#[test]
fn oneof_unresolved_var_defaults_to_i64_for_bitwise() {
    let program = infer_ok(
        r#"
fn mask(x) {
    return x & x
}
"#,
    );
    for stmt in &program.stmts {
        if let TypedStmtKind::Function(func) = &stmt.kind {
            if func.name == "mask" {
                assert_eq!(
                    func.params[0].ty,
                    InferType::I64,
                    "unresolved param constrained by OneOf(integers) via & should default to I64, got {:?}",
                    func.params[0].ty
                );
                return;
            }
        }
    }
    panic!("did not find function 'mask' in typed AST");
}

/// a OneOf-constrained type that was force_dynamic'd by an error should remain Dynamic (no crash, no override)
#[test]
fn oneof_on_dynamic_is_harmless() {
    // should produce a type error (bool is not numeric), but not crash
    assert!(
        sema_err(
            r#"
fn test() {
    let x: bool = true
    let y = x + 1
}
"#
        ),
        "adding bool + int should fail"
    );
}

/// when a concrete type passes OneOf validation and the trial unification binds additional Vars, those bindings should be preserved.
#[test]
fn oneof_temp_subst_merged_on_success() {
    // validates that the OneOf check for + on I64 successfully
    // validates and the program still type-checks correctly
    assert!(
        sema_ok(
            r#"
fn test() -> i64 {
    let x: i64 = 10
    let y: i64 = 20
    return x + y
}
"#
        ),
        "simple add of two i64 should pass"
    );
}

/// ensure that the first matching option is used for merging, and the overall constraint is not broken by the merge.
#[test]
fn oneof_merge_does_not_break_existing_bindings() {
    assert!(
        sema_ok(
            r#"
fn add(a: i32, b: i32) -> i32 {
    return a + b
}

fn test() -> i32 {
    return add(1, 2)
}
"#
        ),
        "i32 arithmetic should pass with OneOf merge"
    );
}

/// the OneOf error path should still work: bool is not numeric
#[test]
fn oneof_still_rejects_invalid_types() {
    assert!(
        sema_err(
            r#"
fn test() -> bool {
    return true + false
}
"#
        ),
        "bool + bool should be rejected by OneOf"
    );
}

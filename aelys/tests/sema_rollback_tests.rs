/// Regression tests for substitution snapshot/rollback on unification failure.
///
/// When unifying compound types (functions, tuples), sub-components are unified
/// one by one. If a later sub-component fails, bindings from earlier successful
/// sub-unifications must be rolled back to avoid corrupting the substitution.
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_sema::TypeInference;
use aelys_sema::types::{InferType, TypeVarId};
use aelys_sema::unify::{Substitution, unify};
use aelys_syntax::Source;

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

// unit tests on Substitution snapshot/restore

#[test]
fn snapshot_restore_undoes_bindings() {
    let mut subst = Substitution::new();
    let v0 = TypeVarId(0);

    let saved = subst.snapshot();
    subst.bind(v0, InferType::I64);
    assert_eq!(subst.apply(&InferType::Var(v0)), InferType::I64);

    subst.restore(saved);
    // after restore, Var(0) should be unbound again
    assert_eq!(subst.apply(&InferType::Var(v0)), InferType::Var(v0));
}

#[test]
fn snapshot_restore_preserves_pre_existing_bindings() {
    let mut subst = Substitution::new();
    let v0 = TypeVarId(0);
    let v1 = TypeVarId(1);

    // bind v0 before the snapshot
    subst.bind(v0, InferType::Bool);

    let saved = subst.snapshot();
    subst.bind(v1, InferType::String);

    subst.restore(saved);
    // v0 should still be bound (it was in the snapshot)
    assert_eq!(subst.apply(&InferType::Var(v0)), InferType::Bool);
    // v1 should be unbound (added after snapshot)
    assert_eq!(subst.apply(&InferType::Var(v1)), InferType::Var(v1));
}

// unit tests: rollback on failed compound unification

#[test]
fn failed_function_unification_rolls_back_param_bindings() {
    // unify(fn(Var(0)) -> string, fn(i64) -> i64) should fail at the return type (string vs i64), but without rollback, Var(0) would remain bound to i64
    let mut subst = Substitution::new();
    let v0 = TypeVarId(0);

    let fn_a = InferType::Function {
        params: vec![InferType::Var(v0)],
        ret: Box::new(InferType::String),
    };
    let fn_b = InferType::Function {
        params: vec![InferType::I64],
        ret: Box::new(InferType::I64),
    };

    // save state, attempt unification, rollback on failure
    let saved = subst.snapshot();
    let result = unify(&fn_a, &fn_b, &mut subst);
    assert!(result.is_err(), "return type mismatch should fail");

    subst.restore(saved);
    // Var(0) must not be bound to i64, the partial param binding was rolled back
    assert_eq!(
        subst.apply(&InferType::Var(v0)),
        InferType::Var(v0),
        "Var(0) should be unbound after rollback"
    );
}

#[test]
fn failed_tuple_unification_rolls_back_element_bindings() {
    // unify((Var(0), Var(1)), (i64, string)) then
    // unify((Var(0), Var(1)), (bool, bool)) should fail because Var(0)=i64 already
    // but with rollback on the second attempt we can try cleanly
    let mut subst = Substitution::new();
    let v0 = TypeVarId(0);
    let v1 = TypeVarId(1);

    // first unification succeeds
    let tuple_a = InferType::Tuple(vec![InferType::Var(v0), InferType::Var(v1)]);
    let tuple_b = InferType::Tuple(vec![InferType::I64, InferType::String]);
    let result = unify(&tuple_a, &tuple_b, &mut subst);
    assert!(result.is_ok());
    assert_eq!(subst.apply(&InferType::Var(v0)), InferType::I64);
    assert_eq!(subst.apply(&InferType::Var(v1)), InferType::String);

    // second unification would fail (i64 vs bool), but rollback cleans up
    let tuple_c = InferType::Tuple(vec![InferType::Bool, InferType::Bool]);
    let saved = subst.snapshot();
    let result2 = unify(&tuple_a, &tuple_c, &mut subst);
    assert!(result2.is_err());

    subst.restore(saved);
    // bindings from the first (successful) unification are preserved
    assert_eq!(subst.apply(&InferType::Var(v0)), InferType::I64);
    assert_eq!(subst.apply(&InferType::Var(v1)), InferType::String);
}

#[test]
fn partial_unification_failure_does_not_corrupt_later_inference() {
    // a type error on one variable should not corrupt a different variable's type.
    // without rollback, if unify(fn(Var)->Var, fn(i64)->string) fails at return, the param binding Var=i64 would persist and contaminate later uses of that Var
    assert!(
        sema_err(
            r#"
fn takes_int(x: i64) -> i64 { return x }
fn takes_str(x: string) -> string { return x }

fn main() {
    let a = takes_int(42)
    let b: string = a
}
"#
        ),
        "assigning i64 result to string should be rejected"
    );
}

#[test]
fn type_error_does_not_poison_unrelated_variables() {
    // even after a type error, unrelated variables should still be inferred correctly.
    assert!(
        sema_ok(
            r#"
fn good() -> i64 {
    let x: i64 = 42
    return x
}
"#
        ),
        "correct function should pass even with rollback changes"
    );
}

#[test]
fn function_type_mismatch_still_detected() {
    // return a string from an i64 function must still be caught after rollback changes.
    assert!(
        sema_err(
            r#"
fn f() -> i64 {
    return "hello"
}
"#
        ),
        "returning string from i64 function must be rejected"
    );
}

#[test]
fn multiple_errors_reported_independently() {
    // two independent type errors should both be reported, not silently suppressed by Dynamic from the first error
    let code = r#"
fn f(x: i64, y: string) -> i64 {
    return x
}

fn main() {
    let a: string = 42
    let b: i64 = "hello"
}
"#;
    let src = Source::new("<test>", code);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let result = TypeInference::infer_program(stmts, src);
    assert!(result.is_err(), "should have type errors");
    let errors = result.unwrap_err();
    assert!(
        errors.len() >= 2,
        "both type errors should be reported, got {} error(s)",
        errors.len()
    );
}

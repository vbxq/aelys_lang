use aelys_sema::env::TypeEnv;
use aelys_sema::types::InferType;
use std::rc::Rc;

#[test]
fn test_define_and_lookup() {
    let mut env = TypeEnv::new();
    env.define_local("x".to_string(), InferType::I64);

    assert_eq!(env.lookup("x"), Some(&InferType::I64));
    assert_eq!(env.lookup("y"), None);
}

#[test]
fn test_nested_scopes() {
    let mut env = TypeEnv::new();
    env.define_local("x".to_string(), InferType::I64);

    env.push_scope();
    env.define_local("y".to_string(), InferType::F64);

    assert_eq!(env.lookup("x"), Some(&InferType::I64));
    assert_eq!(env.lookup("y"), Some(&InferType::F64));

    env.pop_scope();

    assert_eq!(env.lookup("x"), Some(&InferType::I64));
    assert_eq!(env.lookup("y"), None);
}

#[test]
fn test_shadowing() {
    let mut env = TypeEnv::new();
    env.define_local("x".to_string(), InferType::I64);

    env.push_scope();
    env.define_local("x".to_string(), InferType::F64);

    assert_eq!(env.lookup("x"), Some(&InferType::F64));

    env.pop_scope();

    assert_eq!(env.lookup("x"), Some(&InferType::I64));
}

#[test]
fn test_captures() {
    let mut env = TypeEnv::new();
    env.define_capture("captured".to_string(), InferType::Bool);

    assert_eq!(env.lookup("captured"), Some(&InferType::Bool));
}

#[test]
fn test_functions() {
    let mut env = TypeEnv::new();
    let fn_type = InferType::Function {
        params: vec![InferType::I64],
        ret: Box::new(InferType::I64),
    };
    let fn_type_rc = Rc::new(fn_type.clone());
    env.define_function("double".to_string(), fn_type_rc.clone());

    assert_eq!(env.lookup_function("double"), Some(&fn_type_rc));
    assert_eq!(env.lookup("double"), Some(&fn_type));
}

#[test]
fn test_for_closure() {
    let mut env = TypeEnv::new();
    env.define_local("x".to_string(), InferType::I64);
    env.push_scope();
    env.define_local("y".to_string(), InferType::F64);

    let closure_env = env.for_closure();

    assert_eq!(closure_env.lookup("x"), Some(&InferType::I64));
    assert_eq!(closure_env.lookup("y"), Some(&InferType::F64));

    assert_eq!(closure_env.depth(), 1);
}

/// regression test: when a local variable shadows a capture with the same name, for_closure() must preserve the local's type (not the capture's)
/// before that, captures were inserted after locals, overwriting them
#[test]
fn test_for_closure_locals_override_captures() {
    // simulate: outer scope captured x: string, inner function defines local x: i64
    let mut env = TypeEnv::new();
    env.define_capture("x".to_string(), InferType::String);
    env.define_local("x".to_string(), InferType::I64);

    // in the current env, lookup finds local x: i64 (locals searched before captures)
    assert_eq!(env.lookup("x"), Some(&InferType::I64));

    let closure_env = env.for_closure();

    // nested closure must see x as i64 (the local), not string (the capture)
    assert_eq!(
        closure_env.lookup("x"),
        Some(&InferType::I64),
        "for_closure() must give locals priority over captures"
    );
}

/// nested closures with shadowed captures
/// 
/// when multiple levels of nesting each shadow a variable, for_closure() must preserve the innermost type at each level.
#[test]
fn test_for_closure_nested_shadowing() {
    // Level 0: capture x: bool (from grandparent)
    let mut env = TypeEnv::new();
    env.define_capture("x".to_string(), InferType::Bool);
    // Level 0: local x: string shadows the capture
    env.define_local("x".to_string(), InferType::String);
    let mut closure_env_1 = env.for_closure();
    // closure_env_1 should see x: string
    assert_eq!(closure_env_1.lookup("x"), Some(&InferType::String));
    // define local x: i64 in the first closure
    closure_env_1.define_local("x".to_string(), InferType::I64);
    assert_eq!(closure_env_1.lookup("x"), Some(&InferType::I64));
    let closure_env_2 = closure_env_1.for_closure();
    // closure_env_2 should see x: i64 (the local from level 1), not string or bool
    assert_eq!(
        closure_env_2.lookup("x"),
        Some(&InferType::I64),
        "nested for_closure() must preserve innermost local type through nesting levels"
    );
}

#[test]
fn test_mutability_does_not_leak_after_scope_pop() {
    let mut env = TypeEnv::new();
    env.define_local("x".to_string(), InferType::I64);
    assert!(!env.is_mutable("x"));

    env.push_scope();
    env.define_local("x".to_string(), InferType::I64);
    env.mark_mutable("x".to_string());
    assert!(env.is_mutable("x"));
    env.pop_scope();

    assert!(
        !env.is_mutable("x"),
        "inner mutable shadow must not make outer binding mutable"
    );
}

#[test]
fn test_immutable_shadow_hides_mutable_capture() {
    let mut outer = TypeEnv::new();
    outer.define_local("x".to_string(), InferType::I64);
    outer.mark_mutable("x".to_string());

    let mut closure_env = outer.for_closure();
    assert!(
        closure_env.is_mutable("x"),
        "captured mutable x should stay mutable in closure env"
    );

    closure_env.define_local("x".to_string(), InferType::I64);
    assert!(
        !closure_env.is_mutable("x"),
        "immutable local shadow must hide mutable capture"
    );
}

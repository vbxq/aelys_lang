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

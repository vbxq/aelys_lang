use aelys_sema::types::{InferType, TypeVarId};

#[test]
fn test_infer_type_has_vars() {
    assert!(!InferType::Int.has_vars());
    assert!(!InferType::Dynamic.has_vars());
    assert!(InferType::Var(TypeVarId(0)).has_vars());

    let fn_with_var = InferType::Function {
        params: vec![InferType::Int],
        ret: Box::new(InferType::Var(TypeVarId(0))),
    };
    assert!(fn_with_var.has_vars());
}

#[test]
fn test_from_annotation() {
    assert_eq!(InferType::from_annotation("int"), InferType::Int);
    assert_eq!(InferType::from_annotation("float"), InferType::Float);
    assert_eq!(InferType::from_annotation("bool"), InferType::Bool);
    assert_eq!(InferType::from_annotation("string"), InferType::String);
    assert_eq!(InferType::from_annotation("unknown"), InferType::Dynamic);
}

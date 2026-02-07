use aelys_sema::types::{InferType, TypeVarId};
use aelys_syntax::{Span, TypeAnnotation};

fn make_ann(name: &str) -> TypeAnnotation {
    TypeAnnotation::new(name.to_string(), Span::new(0, 0, 1, 1))
}

fn make_generic_ann(name: &str, param: &str) -> TypeAnnotation {
    TypeAnnotation::with_param(
        name.to_string(),
        TypeAnnotation::new(param.to_string(), Span::new(0, 0, 1, 1)),
        Span::new(0, 0, 1, 1),
    )
}

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
    assert_eq!(InferType::from_annotation(&make_ann("int")), InferType::Int);
    assert_eq!(
        InferType::from_annotation(&make_ann("float")),
        InferType::Float
    );
    assert_eq!(
        InferType::from_annotation(&make_ann("bool")),
        InferType::Bool
    );
    assert_eq!(
        InferType::from_annotation(&make_ann("string")),
        InferType::String
    );
    assert_eq!(
        InferType::from_annotation(&make_ann("unknown")),
        InferType::Dynamic
    );
}

#[test]
fn test_from_annotation_generic_types() {
    assert_eq!(
        InferType::from_annotation(&make_generic_ann("array", "int")),
        InferType::Array(Box::new(InferType::Int))
    );

    assert_eq!(
        InferType::from_annotation(&make_generic_ann("vec", "string")),
        InferType::Vec(Box::new(InferType::String))
    );

    // Array<Int> (PascalCase should also work)
    assert_eq!(
        InferType::from_annotation(&make_generic_ann("Array", "Int")),
        InferType::Array(Box::new(InferType::Int))
    );
}

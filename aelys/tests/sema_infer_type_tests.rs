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
    assert!(!InferType::I64.has_vars());
    assert!(!InferType::Dynamic.has_vars());
    assert!(InferType::Var(TypeVarId(0)).has_vars());

    let fn_with_var = InferType::Function {
        params: vec![InferType::I64],
        ret: Box::new(InferType::Var(TypeVarId(0))),
        nogc: false,
    };
    assert!(fn_with_var.has_vars());
}

#[test]
fn test_from_annotation() {
    assert_eq!(InferType::from_annotation(&make_ann("int")), InferType::I64);
    assert_eq!(
        InferType::from_annotation(&make_ann("float")),
        InferType::F64
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
    // "array" (lowercase) is no longer a recognized builtin type annotation.
    // The canonical syntax is [T; N] for fixed-size arrays.
    assert_eq!(
        InferType::from_annotation(&make_generic_ann("array", "int")),
        InferType::Dynamic
    );

    assert_eq!(
        InferType::from_annotation(&make_generic_ann("vec", "string")),
        InferType::Vec(Box::new(InferType::String))
    );

    // PascalCase names are always user-defined types, never builtins.
    // "Array" with a capital A is treated as Struct("Array"), not the builtin array.
    assert_eq!(
        InferType::from_annotation(&make_generic_ann("Array", "Int")),
        InferType::Struct("Array".to_string())
    );
}

#[test]
fn test_from_annotation_sized_array() {
    let inner = TypeAnnotation::new("i64".to_string(), Span::new(0, 0, 1, 1));
    let ann = TypeAnnotation::array_sized(inner, 3, Span::new(0, 0, 1, 1));
    assert_eq!(
        InferType::from_annotation(&ann),
        InferType::Array(Box::new(InferType::I64), Some(3))
    );
}

#[test]
fn test_from_annotation_nested_sized_array() {
    let inner = TypeAnnotation::new("i64".to_string(), Span::new(0, 0, 1, 1));
    let inner_arr = TypeAnnotation::array_sized(inner, 2, Span::new(0, 0, 1, 1));
    let outer = TypeAnnotation::array_sized(inner_arr, 3, Span::new(0, 0, 1, 1));
    assert_eq!(
        InferType::from_annotation(&outer),
        InferType::Array(
            Box::new(InferType::Array(Box::new(InferType::I64), Some(2))),
            Some(3)
        )
    );
}

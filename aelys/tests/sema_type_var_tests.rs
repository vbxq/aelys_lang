use aelys_sema::types::{InferType, TypeVarGen, TypeVarId};

#[test]
fn test_type_var_gen() {
    let mut type_gen = TypeVarGen::new();
    let v1 = type_gen.fresh();
    let v2 = type_gen.fresh();
    let v3 = type_gen.fresh();

    assert_eq!(v1, InferType::Var(TypeVarId(0)));
    assert_eq!(v2, InferType::Var(TypeVarId(1)));
    assert_eq!(v3, InferType::Var(TypeVarId(2)));
    assert_eq!(type_gen.count(), 3);
}

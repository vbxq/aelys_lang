use aelys_sema::types::ResolvedType;

#[test]
fn test_resolved_type_is_integer_ish() {
    assert!(ResolvedType::I64.is_integer_ish());
    assert!(ResolvedType::Uncertain(Box::new(ResolvedType::I64)).is_integer_ish());
    assert!(!ResolvedType::F64.is_integer_ish());
    assert!(!ResolvedType::Dynamic.is_integer_ish());
}

#[test]
fn test_resolved_type_needs_guard() {
    assert!(!ResolvedType::I64.needs_guard());
    assert!(!ResolvedType::Dynamic.needs_guard());
    assert!(ResolvedType::Uncertain(Box::new(ResolvedType::I64)).needs_guard());
}

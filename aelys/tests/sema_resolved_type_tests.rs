use aelys_sema::types::ResolvedType;

#[test]
fn test_resolved_type_is_int_ish() {
    assert!(ResolvedType::Int.is_int_ish());
    assert!(ResolvedType::Uncertain(Box::new(ResolvedType::Int)).is_int_ish());
    assert!(!ResolvedType::Float.is_int_ish());
    assert!(!ResolvedType::Dynamic.is_int_ish());
}

#[test]
fn test_resolved_type_needs_guard() {
    assert!(!ResolvedType::Int.needs_guard());
    assert!(!ResolvedType::Dynamic.needs_guard());
    assert!(ResolvedType::Uncertain(Box::new(ResolvedType::Int)).needs_guard());
}

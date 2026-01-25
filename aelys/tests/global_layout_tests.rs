use aelys_runtime::GlobalLayout;
use std::sync::Arc;

#[test]
fn global_layout_interns_by_names() {
    let a = GlobalLayout::new(vec!["alpha".to_string(), "beta".to_string()]);
    let b = GlobalLayout::new(vec!["alpha".to_string(), "beta".to_string()]);
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(a.id(), b.id());
}

#[test]
fn global_layout_ids_unique_for_different_names() {
    let a = GlobalLayout::new(vec!["alpha".to_string()]);
    let b = GlobalLayout::new(vec!["beta".to_string()]);
    assert_ne!(a.id(), 0);
    assert_ne!(b.id(), 0);
    assert_ne!(a.id(), b.id());
}

#[test]
fn global_layout_empty_is_singleton() {
    let a = GlobalLayout::empty();
    let b = GlobalLayout::empty();
    assert!(Arc::ptr_eq(&a, &b));
    assert_eq!(a.id(), 0);
    assert_eq!(b.id(), 0);
}

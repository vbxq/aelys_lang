//! Tests for Aelys VM objects

use aelys_runtime::{
    AelysFunction, AelysString, Function, GcObject, GcRef, NativeFunction, ObjectKind,
};

#[test]
fn test_aelys_string_creation() {
    let s = AelysString::new("hello");
    assert_eq!(s.as_str(), "hello");
    assert_eq!(s.len(), 5);
    assert!(!s.is_empty());
}

#[test]
fn test_aelys_string_hash() {
    let s1 = AelysString::new("test");
    let s2 = AelysString::new("test");
    let s3 = AelysString::new("different");

    assert_eq!(s1.hash(), s2.hash());
    assert_ne!(s1.hash(), s3.hash());
}

#[test]
fn test_aelys_string_equality() {
    let s1 = AelysString::new("equal");
    let s2 = AelysString::new("equal");
    let s3 = AelysString::new("not equal");

    assert_eq!(s1, s2);
    assert_ne!(s1, s3);
}

#[test]
fn test_gc_ref() {
    let ref1 = GcRef::new(42);
    let ref2 = GcRef::from(42);

    assert_eq!(ref1, ref2);
    assert_eq!(ref1.index(), 42);
    assert_eq!(usize::from(ref1), 42);
}

#[test]
fn test_gc_object() {
    let obj = GcObject::new(ObjectKind::String(AelysString::new("test")));
    assert!(!obj.marked);

    match &obj.kind {
        ObjectKind::String(s) => assert_eq!(s.as_str(), "test"),
        _ => panic!("Expected String variant"),
    }
}

#[test]
fn test_aelys_function() {
    let func = Function::new(Some("test_fn".to_string()), 2);
    let aelys_func = AelysFunction::new(func);

    assert_eq!(aelys_func.name(), Some("test_fn"));
    assert_eq!(aelys_func.arity(), 2);
}

#[test]
fn test_native_function() {
    let native = NativeFunction::new("test", 0);
    assert_eq!(native.name, "test");
    assert_eq!(native.arity, 0);
}

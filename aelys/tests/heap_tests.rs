//! Tests for the Aelys Heap (garbage collector)

use aelys_runtime::{Function, Heap, ObjectKind};

#[test]
fn test_heap_creation() {
    let heap = Heap::new();
    assert_eq!(heap.object_count(), 0);
    assert_eq!(heap.bytes_allocated(), 0);
    assert!(heap.next_gc_threshold() > 0);
}

#[test]
fn test_alloc_string() {
    let mut heap = Heap::new();
    let gc_ref = heap.alloc_string("hello");

    assert_eq!(heap.object_count(), 1);
    assert!(heap.bytes_allocated() > 0);

    let obj = heap.get(gc_ref).expect("Object should exist");
    match &obj.kind {
        ObjectKind::String(s) => {
            assert_eq!(s.as_str(), "hello");
        }
        _ => panic!("Expected String object"),
    }
}

#[test]
fn test_intern_string() {
    let mut heap = Heap::new();

    // Intern same string twice
    let ref1 = heap.intern_string("test");
    let ref2 = heap.intern_string("test");

    // Should return same reference
    assert_eq!(ref1, ref2);
    assert_eq!(heap.object_count(), 1);
}

#[test]
fn test_intern_different_strings() {
    let mut heap = Heap::new();

    let ref1 = heap.intern_string("hello");
    let ref2 = heap.intern_string("world");

    // Should be different references
    assert_ne!(ref1, ref2);
    assert_eq!(heap.object_count(), 2);
}

#[test]
fn test_alloc_function() {
    let mut heap = Heap::new();
    let func = Function::new(Some("test".to_string()), 2);
    let gc_ref = heap.alloc_function(func);

    let obj = heap.get(gc_ref).expect("Object should exist");
    match &obj.kind {
        ObjectKind::Function(f) => {
            assert_eq!(f.name(), Some("test"));
            assert_eq!(f.arity(), 2);
        }
        _ => panic!("Expected Function object"),
    }
}

#[test]
fn test_alloc_native() {
    let mut heap = Heap::new();
    let gc_ref = heap.alloc_native("test_fn", 0);

    let obj = heap.get(gc_ref).expect("Object should exist");
    match &obj.kind {
        ObjectKind::Native(n) => {
            assert_eq!(n.name, "test_fn");
            assert_eq!(n.arity, 0);
        }
        _ => panic!("Expected Native object"),
    }
}

#[test]
fn test_mark_and_sweep() {
    let mut heap = Heap::new();

    // Allocate some objects
    let ref1 = heap.alloc_string("keep me");
    let _ref2 = heap.alloc_string("free me");
    let ref3 = heap.alloc_string("keep me too");

    assert_eq!(heap.object_count(), 3);
    let initial_bytes = heap.bytes_allocated();

    // Mark only ref1 and ref3 as reachable
    heap.mark(ref1);
    heap.mark(ref3);

    // Sweep should free ref2
    let freed = heap.sweep();

    assert_eq!(freed, 1);
    assert_eq!(heap.object_count(), 2);
    assert!(heap.bytes_allocated() < initial_bytes);

    // Verify kept objects are still accessible
    assert!(heap.get(ref1).is_some());
    assert!(heap.get(ref3).is_some());
}

#[test]
fn test_gc_threshold() {
    let mut heap = Heap::new();

    let initial_threshold = heap.next_gc_threshold();

    // Allocate until we approach threshold
    for _ in 0..100 {
        heap.alloc_string("test string to fill heap");
    }

    // Should consider collecting
    if heap.should_collect() {
        // After sweep, threshold should grow
        heap.sweep();
        assert!(heap.next_gc_threshold() >= heap.bytes_allocated());
    }

    assert!(heap.next_gc_threshold() >= initial_threshold);
}

#[test]
fn test_free_list_reuse() {
    let mut heap = Heap::new();

    // Allocate objects
    let ref1 = heap.alloc_string("first");
    let ref2 = heap.alloc_string("second");

    assert_eq!(heap.object_count(), 2);

    // Mark only ref1, sweep ref2
    heap.mark(ref1);
    heap.sweep();

    assert_eq!(heap.object_count(), 1);

    // Allocate new object - should reuse freed slot
    let ref3 = heap.alloc_string("third");

    // ref3 should have reused ref2's slot
    assert_eq!(ref3.index(), ref2.index());
    assert_eq!(heap.object_count(), 2);
}

#[test]
fn test_fnv1a_hash() {
    // Test FNV-1a hash function
    let hash1 = Heap::fnv1a_hash(b"hello");
    let hash2 = Heap::fnv1a_hash(b"hello");
    let hash3 = Heap::fnv1a_hash(b"world");

    // Same input produces same hash
    assert_eq!(hash1, hash2);

    // Different input produces different hash (usually)
    assert_ne!(hash1, hash3);
}

#[test]
fn test_get_mut() {
    let mut heap = Heap::new();
    let gc_ref = heap.alloc_string("test");

    // Get mutable reference and mark
    if let Some(obj) = heap.get_mut(gc_ref) {
        assert!(!obj.marked);
        obj.marked = true;
    }

    // Verify mark persisted
    let obj = heap.get(gc_ref).unwrap();
    assert!(obj.marked);
}

#[test]
fn test_sweep_updates_threshold() {
    let mut heap = Heap::new();

    // Allocate and free all objects
    let _ref1 = heap.alloc_string("benchmarks");

    // Sweep without marking anything
    heap.sweep();

    // All objects freed
    assert_eq!(heap.object_count(), 0);
    assert_eq!(heap.bytes_allocated(), 0);

    // Threshold should be at least initial
    assert!(heap.next_gc_threshold() >= Heap::INITIAL_GC_THRESHOLD);
}

//! Unit tests for ManualHeap struct.

use aelys_runtime::Value;
use aelys_runtime::manual_heap::{ManualHeap, ManualHeapError};

#[test]
fn test_alloc_basic() {
    let mut heap = ManualHeap::new();
    let h = heap.alloc(10, 0).unwrap();
    assert_eq!(heap.load(h, 0).unwrap(), Value::null());
    assert_eq!(heap.size(h).unwrap(), 10);
}

#[test]
fn test_store_load() {
    let mut heap = ManualHeap::new();
    let h = heap.alloc(10, 0).unwrap();
    heap.store(h, 5, Value::int(42)).unwrap();
    assert_eq!(heap.load(h, 5).unwrap().as_int(), Some(42));
}

#[test]
fn test_alloc_zero_fails() {
    let mut heap = ManualHeap::new();
    assert!(matches!(
        heap.alloc(0, 0),
        Err(ManualHeapError::InvalidSize)
    ));
}

#[test]
fn test_double_free() {
    let mut heap = ManualHeap::new();
    let h = heap.alloc(10, 0).unwrap();
    heap.free(h, 1).unwrap();
    assert!(matches!(
        heap.free(h, 2),
        Err(ManualHeapError::DoubleFree { .. })
    ));
}

#[test]
fn test_use_after_free_load() {
    let mut heap = ManualHeap::new();
    let h = heap.alloc(10, 0).unwrap();
    heap.free(h, 1).unwrap();
    assert!(matches!(
        heap.load(h, 0),
        Err(ManualHeapError::UseAfterFree { .. })
    ));
}

#[test]
fn test_use_after_free_store() {
    let mut heap = ManualHeap::new();
    let h = heap.alloc(10, 0).unwrap();
    heap.free(h, 1).unwrap();
    assert!(matches!(
        heap.store(h, 0, Value::int(1)),
        Err(ManualHeapError::UseAfterFree { .. })
    ));
}

#[test]
fn test_out_of_bounds() {
    let mut heap = ManualHeap::new();
    let h = heap.alloc(5, 0).unwrap();
    assert!(matches!(
        heap.load(h, 10),
        Err(ManualHeapError::OutOfBounds {
            offset: 10,
            size: 5
        })
    ));
    assert!(matches!(
        heap.store(h, 10, Value::int(1)),
        Err(ManualHeapError::OutOfBounds {
            offset: 10,
            size: 5
        })
    ));
}

#[test]
fn test_invalid_handle() {
    let heap = ManualHeap::new();
    assert!(matches!(
        heap.load(999, 0),
        Err(ManualHeapError::InvalidHandle)
    ));
}

#[test]
fn test_free_reuses_slots() {
    let mut heap = ManualHeap::new();
    let h1 = heap.alloc(10, 0).unwrap();
    heap.free(h1, 1).unwrap();
    let h2 = heap.alloc(5, 2).unwrap();
    assert_eq!(h1, h2); // Same slot reused
}

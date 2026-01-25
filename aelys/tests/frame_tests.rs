//! Tests for Aelys VM call frames

use aelys_runtime::{CallFrame, GcRef};
use std::ptr;

#[test]
fn test_call_frame_creation() {
    let gc_ref = GcRef::new(5);
    let frame = CallFrame::new(gc_ref, 10, ptr::null(), 0, ptr::null(), 0, 0);

    assert_eq!(frame.function(), gc_ref);
    assert_eq!(frame.ip(), 0);
    assert_eq!(frame.base(), 10);
}

#[test]
fn test_advance_ip() {
    let gc_ref = GcRef::new(0);
    let mut frame = CallFrame::new(gc_ref, 0, ptr::null(), 0, ptr::null(), 0, 0);

    assert_eq!(frame.ip(), 0);

    frame.advance_ip();
    assert_eq!(frame.ip(), 1);

    frame.advance_ip();
    assert_eq!(frame.ip(), 2);
}

#[test]
fn test_set_ip() {
    let gc_ref = GcRef::new(0);
    let mut frame = CallFrame::new(gc_ref, 0, ptr::null(), 0, ptr::null(), 0, 0);

    frame.set_ip(42);
    assert_eq!(frame.ip(), 42);

    frame.set_ip(0);
    assert_eq!(frame.ip(), 0);
}

#[test]
fn test_jump_forward() {
    let gc_ref = GcRef::new(0);
    let mut frame = CallFrame::new(gc_ref, 0, ptr::null(), 0, ptr::null(), 0, 0);

    frame.set_ip(10);
    frame.jump(5);
    assert_eq!(frame.ip(), 15);
}

#[test]
fn test_jump_backward() {
    let gc_ref = GcRef::new(0);
    let mut frame = CallFrame::new(gc_ref, 0, ptr::null(), 0, ptr::null(), 0, 0);

    frame.set_ip(10);
    frame.jump(-3);
    assert_eq!(frame.ip(), 7);
}

#[test]
fn test_jump_backward_saturating() {
    let gc_ref = GcRef::new(0);
    let mut frame = CallFrame::new(gc_ref, 0, ptr::null(), 0, ptr::null(), 0, 0);

    frame.set_ip(2);
    frame.jump(-10);
    assert_eq!(frame.ip(), 0); // Saturates at 0
}

#[test]
fn test_register_index() {
    let gc_ref = GcRef::new(0);
    let frame = CallFrame::new(gc_ref, 100, ptr::null(), 0, ptr::null(), 0, 0);

    assert_eq!(frame.register_index(0), Some(100));
    assert_eq!(frame.register_index(1), Some(101));
    assert_eq!(frame.register_index(5), Some(105));
    assert_eq!(frame.register_index(255), Some(355));
}

#[test]
fn test_register_index_overflow() {
    let gc_ref = GcRef::new(0);
    let frame = CallFrame::new(gc_ref, usize::MAX - 10, ptr::null(), 0, ptr::null(), 0, 0);

    assert_eq!(frame.register_index(10), Some(usize::MAX));
    assert_eq!(frame.register_index(11), None);
    assert_eq!(frame.register_index(255), None);
}

#[test]
fn test_clone() {
    let gc_ref = GcRef::new(42);
    let frame1 = CallFrame::new(gc_ref, 10, ptr::null(), 0, ptr::null(), 0, 0);
    let mut frame2 = frame1.clone();

    assert_eq!(frame1.function(), frame2.function());
    assert_eq!(frame1.ip(), frame2.ip());
    assert_eq!(frame1.base(), frame2.base());

    // Modifying clone doesn't affect original
    frame2.advance_ip();
    assert_eq!(frame1.ip(), 0);
    assert_eq!(frame2.ip(), 1);
}

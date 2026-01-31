mod common;
use common::{assert_aelys_bool, assert_aelys_int, assert_aelys_null, assert_aelys_str, run_aelys_err, run_aelys_ok};

#[test]
fn test_bytes_alloc_and_size() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(100)
        bytes.size(buf)
    "#,
        100,
    );
}

#[test]
fn test_bytes_alloc_zero_fails() {
    let err = run_aelys_err(
        r#"
        needs std.bytes
        bytes.alloc(0)
    "#,
    );
    assert!(
        err.contains("positive") || err.contains("must be"),
        "Expected error about positive size, got: {}",
        err
    );
}

#[test]
fn test_bytes_alloc_negative_fails() {
    let err = run_aelys_err(
        r#"
        needs std.bytes
        bytes.alloc(-10)
    "#,
    );
    assert!(
        err.contains("positive") || err.contains("must be"),
        "Expected error about positive size, got: {}",
        err
    );
}

#[test]
fn test_bytes_free() {
    assert_aelys_null(
        r#"
        needs std.bytes
        let buf = bytes.alloc(100)
        bytes.free(buf)
    "#,
    );
}

#[test]
fn test_bytes_free_null_is_noop() {
    assert_aelys_null(
        r#"
        needs std.bytes
        bytes.free(null)
    "#,
    );
}

#[test]
fn test_bytes_read_write_u8() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.write_u8(buf, 0, 42)
        bytes.read_u8(buf, 0)
    "#,
        42,
    );
}

#[test]
fn test_bytes_u8_range() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.write_u8(buf, 0, 0)
        bytes.write_u8(buf, 1, 255)
        bytes.read_u8(buf, 0) + bytes.read_u8(buf, 1)
    "#,
        255,
    );
}

#[test]
fn test_bytes_u8_out_of_range() {
    let err = run_aelys_err(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.write_u8(buf, 0, 256)
    "#,
    );
    assert!(
        err.contains("range") || err.contains("0, 255"),
        "Expected error about u8 range, got: {}",
        err
    );
}

#[test]
fn test_bytes_u8_out_of_bounds() {
    let err = run_aelys_err(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.read_u8(buf, 10)
    "#,
    );
    assert!(
        err.contains("offset") && err.contains("size"),
        "Expected bounds error, got: {}",
        err
    );
}

#[test]
fn test_bytes_read_write_u16() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.write_u16(buf, 0, 12345)
        bytes.read_u16(buf, 0)
    "#,
        12345,
    );
}

#[test]
fn test_bytes_u16_little_endian() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.write_u16(buf, 0, 258)
        bytes.read_u8(buf, 0)
    "#,
        2,
    );
}

#[test]
fn test_bytes_read_write_u32() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.write_u32(buf, 0, 123456789)
        bytes.read_u32(buf, 0)
    "#,
        123456789,
    );
}

#[test]
fn test_bytes_u32_out_of_bounds() {
    let err = run_aelys_err(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.read_u32(buf, 8)
    "#,
    );
    assert!(
        err.contains("exceeds") || err.contains("offset"),
        "Expected bounds error, got: {}",
        err
    );
}

#[test]
fn test_bytes_read_write_u64() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(16)
        bytes.write_u64(buf, 0, 140737488355327)
        bytes.read_u64(buf, 0)
    "#,
        140737488355327_i64,
    );
}

#[test]
fn test_bytes_read_write_f32() {
    let result = run_aelys_ok(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.write_f32(buf, 0, 3.14)
        bytes.read_f32(buf, 0)
    "#,
    );
    let val = result.as_float().expect("Expected float result");
    assert!(
        (val - 3.14).abs() < 0.01,
        "Expected value close to 3.14, got {}",
        val
    );
}

#[test]
fn test_bytes_read_write_f64() {
    let result = run_aelys_ok(
        r#"
        needs std.bytes
        let buf = bytes.alloc(16)
        bytes.write_f64(buf, 0, 3.141592653589793)
        bytes.read_f64(buf, 0)
    "#,
    );
    let val = result.as_float().expect("Expected float result");
    assert!(
        (val - std::f64::consts::PI).abs() < 1e-15,
        "Expected value close to PI, got {}",
        val
    );
}

#[test]
fn test_bytes_fill() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.fill(buf, 2, 5, 42)
        bytes.read_u8(buf, 0) + bytes.read_u8(buf, 2) + bytes.read_u8(buf, 6)
    "#,
        84,
    );
}

#[test]
fn test_bytes_copy_same_buffer() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(20)
        bytes.write_u8(buf, 0, 11)
        bytes.write_u8(buf, 1, 22)
        bytes.write_u8(buf, 2, 33)
        bytes.copy(buf, 0, buf, 10, 3)
        bytes.read_u8(buf, 10) + bytes.read_u8(buf, 11) + bytes.read_u8(buf, 12)
    "#,
        66,
    );
}

#[test]
fn test_bytes_copy_different_buffers() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let src = bytes.alloc(10)
        let dst = bytes.alloc(10)
        bytes.write_u8(src, 0, 100)
        bytes.write_u8(src, 1, 200)
        bytes.copy(src, 0, dst, 5, 2)
        bytes.read_u8(dst, 5) + bytes.read_u8(dst, 6)
    "#,
        300,
    );
}

#[test]
fn test_bytes_invalid_handle() {
    let err = run_aelys_err(
        r#"
        needs std.bytes
        bytes.read_u8(99999, 0)
    "#,
    );
    assert!(
        err.contains("invalid") || err.contains("handle"),
        "Expected invalid handle error, got: {}",
        err
    );
}

#[test]
fn test_bytes_use_after_free() {
    let err = run_aelys_err(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.free(buf)
        bytes.read_u8(buf, 0)
    "#,
    );
    assert!(
        err.contains("invalid") || err.contains("handle"),
        "Expected invalid handle error after free, got: {}",
        err
    );
}

#[test]
fn test_bytes_negative_offset() {
    let err = run_aelys_err(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.read_u8(buf, -1)
    "#,
    );
    assert!(
        err.contains("negative"),
        "Expected error about negative offset, got: {}",
        err
    );
}

#[test]
fn test_bytes_resize() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.write_u8(buf, 0, 42)
        bytes.resize(buf, 20)
        bytes.size(buf)
    "#,
        20,
    );
}

#[test]
fn test_bytes_resize_preserves_data() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.write_u8(buf, 0, 42)
        bytes.resize(buf, 20)
        bytes.read_u8(buf, 0)
    "#,
        42,
    );
}

#[test]
fn test_bytes_clone() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.write_u8(buf, 0, 42)
        let buf2 = bytes.clone(buf)
        bytes.write_u8(buf, 0, 0)
        bytes.read_u8(buf2, 0)
    "#,
        42,
    );
}

#[test]
fn test_bytes_equals() {
    assert_aelys_bool(
        r#"
        needs std.bytes
        let a = bytes.alloc(4)
        let b = bytes.alloc(4)
        bytes.write_u32(a, 0, 12345)
        bytes.write_u32(b, 0, 12345)
        bytes.equals(a, b)
    "#,
        true,
    );
}

#[test]
fn test_bytes_equals_different() {
    assert_aelys_bool(
        r#"
        needs std.bytes
        let a = bytes.alloc(4)
        let b = bytes.alloc(4)
        bytes.write_u32(a, 0, 12345)
        bytes.write_u32(b, 0, 54321)
        bytes.equals(a, b)
    "#,
        false,
    );
}

#[test]
fn test_bytes_signed_i8() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(4)
        bytes.write_i8(buf, 0, -128)
        bytes.write_i8(buf, 1, 127)
        bytes.read_i8(buf, 0) + bytes.read_i8(buf, 1)
    "#,
        -1,
    );
}

#[test]
fn test_bytes_signed_i16() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(4)
        bytes.write_i16(buf, 0, -32768)
        bytes.read_i16(buf, 0)
    "#,
        -32768,
    );
}

#[test]
fn test_bytes_signed_i32() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(8)
        bytes.write_i32(buf, 0, -2147483648)
        bytes.read_i32(buf, 0)
    "#,
        -2147483648,
    );
}

#[test]
fn test_bytes_big_endian_u16() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(4)
        bytes.write_u16_be(buf, 0, 0x1234)
        bytes.read_u8(buf, 0)
    "#,
        0x12,
    );
}

#[test]
fn test_bytes_big_endian_u32() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(8)
        bytes.write_u32_be(buf, 0, 0x12345678)
        bytes.read_u8(buf, 0)
    "#,
        0x12,
    );
}

#[test]
fn test_bytes_big_endian_roundtrip() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(8)
        bytes.write_u32_be(buf, 0, 0xDEADBEEF)
        bytes.read_u32_be(buf, 0)
    "#,
        0xDEADBEEF_u32 as i64,
    );
}

#[test]
fn test_bytes_from_string() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.from_string("Hello")
        bytes.size(buf)
    "#,
        5,
    );
}

#[test]
fn test_bytes_from_string_content() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.from_string("ABC")
        bytes.read_u8(buf, 0) + bytes.read_u8(buf, 1) + bytes.read_u8(buf, 2)
    "#,
        65 + 66 + 67,
    );
}

#[test]
fn test_bytes_decode() {
    assert_aelys_str(
        r#"
        needs std.bytes
        let buf = bytes.alloc(5)
        bytes.write_u8(buf, 0, 72)
        bytes.write_u8(buf, 1, 101)
        bytes.write_u8(buf, 2, 108)
        bytes.write_u8(buf, 3, 108)
        bytes.write_u8(buf, 4, 111)
        bytes.decode(buf, 0, 5)
    "#,
        "Hello",
    );
}

#[test]
fn test_bytes_write_string() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(10)
        bytes.write_string(buf, 0, "Hi")
        bytes.read_u8(buf, 0) + bytes.read_u8(buf, 1)
    "#,
        72 + 105,
    );
}

#[test]
fn test_bytes_write_string_returns_length() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(20)
        bytes.write_string(buf, 0, "Hello, World!")
    "#,
        13,
    );
}

#[test]
fn test_bytes_find() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.from_string("Hello, World!")
        bytes.find(buf, 0, -1, 111)
    "#,
        4,
    );
}

#[test]
fn test_bytes_find_not_found() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.from_string("Hello")
        bytes.find(buf, 0, -1, 120)
    "#,
        -1,
    );
}

#[test]
fn test_bytes_find_with_range() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.from_string("abcabc")
        bytes.find(buf, 3, 6, 97)
    "#,
        3,
    );
}

#[test]
fn test_bytes_reverse() {
    assert_aelys_str(
        r#"
        needs std.bytes
        let buf = bytes.from_string("Hello")
        bytes.reverse(buf, 0, 5)
        bytes.decode(buf, 0, 5)
    "#,
        "olleH",
    );
}

#[test]
fn test_bytes_swap() {
    assert_aelys_int(
        r#"
        needs std.bytes
        let buf = bytes.alloc(4)
        bytes.write_u8(buf, 0, 10)
        bytes.write_u8(buf, 3, 40)
        bytes.swap(buf, 0, 3)
        bytes.read_u8(buf, 0) + bytes.read_u8(buf, 3)
    "#,
        50,
    );
}

#[test]
fn test_bytes_f32_big_endian() {
    let result = run_aelys_ok(
        r#"
        needs std.bytes
        let buf = bytes.alloc(8)
        bytes.write_f32_be(buf, 0, 3.14)
        bytes.read_f32_be(buf, 0)
    "#,
    );
    let val = result.as_float().expect("Expected float result");
    assert!(
        (val - 3.14).abs() < 0.01,
        "Expected value close to 3.14, got {}",
        val
    );
}

#[test]
fn test_bytes_f64_big_endian() {
    let result = run_aelys_ok(
        r#"
        needs std.bytes
        let buf = bytes.alloc(16)
        bytes.write_f64_be(buf, 0, 3.141592653589793)
        bytes.read_f64_be(buf, 0)
    "#,
    );
    let val = result.as_float().expect("Expected float result");
    assert!(
        (val - std::f64::consts::PI).abs() < 1e-15,
        "Expected value close to PI, got {}",
        val
    );
}

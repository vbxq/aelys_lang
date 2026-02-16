mod common;
use common::*;

#[test]
fn string_len_basic() {
    let code = r#"
"hello".len()
"#;
    assert_aelys_int(code, 5);
}

#[test]
fn string_len_empty() {
    let code = r#"
"".len()
"#;
    assert_aelys_int(code, 0);
}

#[test]
fn string_char_len_ascii() {
    let code = r#"
"hello".char_len()
"#;
    assert_aelys_int(code, 5);
}

#[test]
fn string_char_len_unicode() {
    let code = r#"
"héllo".char_len()
"#;
    assert_aelys_int(code, 5);
}

#[test]
fn string_char_at_valid() {
    let code = r#"
let c = "hello".char_at(1)
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_char_at_negative() {
    let code = r#"
let c = "hello".char_at(-1)
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_char_at_out_of_bounds() {
    let code = r#"
let c = "hello".char_at(100)
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_byte_at_valid() {
    let code = r#"
"ABC".byte_at(0)
"#;
    assert_aelys_int(code, 65); // 'A'
}

#[test]
fn string_byte_at_out_of_bounds() {
    let code = r#"
"hi".byte_at(10)
"#;
    assert_aelys_int(code, -1);
}

#[test]
fn string_byte_at_negative() {
    let code = r#"
"test".byte_at(-1)
"#;
    assert_aelys_int(code, -1);
}

#[test]
fn string_substr_basic() {
    let code = r#"
let s = "hello world".substr(0, 5)
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_substr_negative_start() {
    let code = r#"
let s = "hello".substr(-1, 3)
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_substr_negative_len() {
    let code = r#"
let s = "hello".substr(0, -5)
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_to_upper() {
    let code = r#"
let s = "hello".to_upper()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_to_lower() {
    let code = r#"
let s = "HELLO".to_lower()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_capitalize() {
    let code = r#"
let s = "hello".capitalize()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_capitalize_empty() {
    let code = r#"
let s = "".capitalize()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_contains_true() {
    let code = r#"
if "hello world".contains("wor") { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_contains_false() {
    let code = r#"
if "hello".contains("xyz") { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_starts_with_true() {
    let code = r#"
if "hello".starts_with("he") { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_starts_with_false() {
    let code = r#"
if "hello".starts_with("lo") { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_ends_with_true() {
    let code = r#"
if "hello".ends_with("lo") { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_ends_with_false() {
    let code = r#"
if "hello".ends_with("he") { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_find_exists() {
    let code = r#"
let pos = "hello world".find("wor")
if pos >= 0 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_find_not_found() {
    let code = r#"
"hello".find("xyz")
"#;
    assert_aelys_int(code, -1);
}

#[test]
fn string_rfind_exists() {
    let code = r#"
let pos = "hello hello".rfind("hello")
if pos > 0 { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_rfind_not_found() {
    let code = r#"
"hello".rfind("xyz")
"#;
    assert_aelys_int(code, -1);
}

#[test]
fn string_count_occurrences() {
    let code = r#"
"hello hello hello".count("hello")
"#;
    assert_aelys_int(code, 3);
}

#[test]
fn string_count_zero() {
    let code = r#"
"hello".count("xyz")
"#;
    assert_aelys_int(code, 0);
}

#[test]
fn string_replace_all() {
    let code = r#"
let s = "hello hello".replace("hello", "hi")
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_replace_first() {
    let code = r#"
let s = "hello hello".replace_first("hello", "hi")
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_split_basic() {
    let code = r#"
let parts = "a,b,c".split(",")
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_split_empty_separator() {
    let code = r#"
let parts = "abc".split("")
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_join_basic() {
    let code = r#"
let parts = "a\nb\nc"
let s = parts.join(",")
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_repeat_positive() {
    let code = r#"
let s = "ab".repeat(3)
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_repeat_zero() {
    let code = r#"
let s = "abc".repeat(0)
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_repeat_negative() {
    let code = r#"
let s = "abc".repeat(-5)
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_reverse_basic() {
    let code = r#"
let s = "abc".reverse()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_reverse_unicode() {
    let code = r#"
let s = "héllo".reverse()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_concat_basic() {
    let code = r#"
let s = "hello".concat(" world")
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_trim_whitespace() {
    let code = r#"
let s = "  hello  ".trim()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_trim_start() {
    let code = r#"
let s = "  hello".trim_start()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_trim_end() {
    let code = r#"
let s = "hello  ".trim_end()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_pad_left_basic() {
    let code = r#"
let s = "5".pad_left(3, "0")
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_pad_left_already_wide() {
    let code = r#"
let s = "hello".pad_left(2, "x")
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_pad_right_basic() {
    let code = r#"
let s = "hi".pad_right(5, ".")
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_is_empty_true() {
    let code = r#"
if "".is_empty() { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_is_empty_false() {
    let code = r#"
if "x".is_empty() { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_is_whitespace_true() {
    let code = r#"
if "   ".is_whitespace() { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_is_whitespace_false() {
    let code = r#"
if " a ".is_whitespace() { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_is_whitespace_empty() {
    let code = r#"
if "".is_whitespace() { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_is_numeric_true() {
    let code = r#"
if "12345".is_numeric() { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_is_numeric_false() {
    let code = r#"
if "12a34".is_numeric() { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_is_alphabetic_true() {
    let code = r#"
if "hello".is_alphabetic() { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_is_alphabetic_false() {
    let code = r#"
if "hello123".is_alphabetic() { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_is_alphanumeric_true() {
    let code = r#"
if "hello123".is_alphanumeric() { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_is_alphanumeric_false() {
    let code = r#"
if "hello-123".is_alphanumeric() { 0 } else { 1 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_lines_basic() {
    let code = r#"
let s = "a\nb\nc".lines()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_line_count() {
    let code = r#"
"a\nb\nc".line_count()
"#;
    assert_aelys_int(code, 3);
}

#[test]
fn string_line_count_empty() {
    let code = r#"
"".line_count()
"#;
    assert_aelys_int(code, 0);
}

#[test]
fn string_bytes_basic() {
    let code = r#"
let b = "AB".bytes()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_chars_basic() {
    let code = r#"
let c = "abc".chars()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_unicode_length_mismatch() {
    let code = r#"
let byte_len = "😀".len()
let char_len = "😀".char_len()
if byte_len > char_len { 1 } else { 0 }
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_emoji_reverse() {
    let code = r#"
let s = "😀😁".reverse()
42
"#;
    assert_aelys_int(code, 42);
}

#[test]
fn string_rtl_text() {
    let code = r#"
let s = "مرحبا".reverse()
42
"#;
    assert_aelys_int(code, 42);
}

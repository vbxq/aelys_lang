mod common;
use common::*;

// String Indexing Tests (stuff[i])
#[test]
fn string_index_first_char() {
    let code = r#"
let s = "hello"
let c = s[0]
c.char_len()
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_index_last_char() {
    let code = r#"
let s = "abcde"
let c = s[4]
c.char_len()
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_index_accumulate_with_range_for() {
    let code = r#"
let s = "hello"
let mut count = 0
for i in 0..s.char_len() {
    let c = s[i]
    count++
}
count
"#;
    assert_aelys_int(code, 5);
}

#[test]
fn string_index_returns_single_char_string() {
    let code = r#"
let s = "abc"
let c = s[1]
c
"#;
    assert_aelys_str(code, "b");
}

#[test]
fn string_index_first() {
    let code = r#"
let s = "xyz"
s[0]
"#;
    assert_aelys_str(code, "x");
}

#[test]
fn string_index_middle() {
    let code = r#"
let s = "world"
s[2]
"#;
    assert_aelys_str(code, "r");
}

// =============================================================================
// String ForEach Iteration Tests (for letter in stuff)
// =============================================================================

#[test]
fn string_foreach_count_chars() {
    let code = r#"
let mut count = 0
for c in "hello" {
    count++
}
count
"#;
    assert_aelys_int(code, 5);
}

#[test]
fn string_foreach_with_variable() {
    let code = r#"
let stuff = "abcdef"
let mut count = 0
for letter in stuff {
    count++
}
count
"#;
    assert_aelys_int(code, 6);
}

#[test]
fn string_foreach_empty_string() {
    let code = r#"
let mut count = 0
for c in "" {
    count++
}
count
"#;
    assert_aelys_int(code, 0);
}

#[test]
fn string_foreach_single_char() {
    let code = r#"
let mut count = 0
for c in "x" {
    count++
}
count
"#;
    assert_aelys_int(code, 1);
}

#[test]
fn string_foreach_first_char() {
    let code = r#"
let mut result = ""
for c in "hello" {
    result = c
    break
}
result
"#;
    assert_aelys_str(code, "h");
}

#[test]
fn string_foreach_in_function() {
    let code = r#"
fn count_chars(s: string) -> int {
    let mut n = 0
    for c in s {
        n++
    }
    return n
}
count_chars("testing")
"#;
    assert_aelys_int(code, 7);
}

#[test]
fn string_foreach_nested_in_range_for() {
    let code = r#"
let mut total = 0
for i in 0..3 {
    for c in "ab" {
        total++
    }
}
total
"#;
    assert_aelys_int(code, 6);
}

#[test]
fn string_foreach_with_conditional() {
    let code = r#"
let mut count = 0
for c in "aAbBcC" {
    if c == "a" or c == "b" or c == "c" {
        count++
    }
}
count
"#;
    assert_aelys_int(code, 3);
}

// =============================================================================
// String Indexing + ForEach Consistency
// =============================================================================

#[test]
fn string_index_and_foreach_same_count() {
    let code = r#"
let s = "hello world"
let mut count_index = 0
for i in 0..s.char_len() {
    let c = s[i]
    count_index++
}
let mut count_foreach = 0
for c in s {
    count_foreach++
}
if count_index == count_foreach { count_index } else { -1 }
"#;
    assert_aelys_int(code, 11);
}

// =============================================================================
// Unicode Handling
// =============================================================================

#[test]
fn string_foreach_unicode_accented() {
    let code = r#"
let mut count = 0
for c in "café" {
    count++
}
count
"#;
    assert_aelys_int(code, 4);
}

#[test]
fn string_foreach_unicode_multibyte() {
    let code = r#"
let mut count = 0
for c in "héllo" {
    count++
}
count
"#;
    assert_aelys_int(code, 5);
}

// =============================================================================
// ForEach with break
// =============================================================================

#[test]
fn string_foreach_break_early() {
    let code = r#"
let mut count = 0
for c in "abcdefgh" {
    count++
    if count == 3 {
        break
    }
}
count
"#;
    assert_aelys_int(code, 3);
}

// =============================================================================
// Indexing on string literal
// =============================================================================

#[test]
fn string_literal_index() {
    let code = r#"
"hello"[0]
"#;
    assert_aelys_str(code, "h");
}

#[test]
fn string_literal_index_last() {
    let code = r#"
"world"[4]
"#;
    assert_aelys_str(code, "d");
}

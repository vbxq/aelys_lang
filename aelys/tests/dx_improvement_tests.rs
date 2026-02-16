mod common;

use common::{assert_aelys_bool, assert_aelys_int, assert_aelys_str, run_aelys};

#[test]
fn test_print_no_newline() {
    let result = run_aelys(r#"print("hello"); 42"#);
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_println_works() {
    let result = run_aelys(r#"println("hello"); 42"#);
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_for_each_vec_int() {
    assert_aelys_int(
        r#"
        let v = Vec[1, 2, 3]
        let mut sum = 0
        for item in v {
            sum += item
        }
        sum
        "#,
        6,
    );
}

#[test]
fn test_for_each_vec_string() {
    assert_aelys_int(
        r#"
        let v = Vec[]
        v.push("a")
        v.push("b")
        v.push("c")
        let mut count = 0
        for item in v {
            count++
        }
        count
        "#,
        3,
    );
}

#[test]
fn test_for_each_vec_float() {
    let result = run_aelys(
        r#"
        let v = Vec[1.0, 2.0, 3.0]
        let mut sum = 0.0
        for x in v {
            sum += x
        }
        sum
        "#,
    );
    assert_eq!(result.as_float(), Some(6.0));
}

#[test]
fn test_for_each_vec_in_function() {
    assert_aelys_int(
        r#"
        fn sum_vec(v) {
            let mut total = 0
            for item in v {
                total += item
            }
            return total
        }
        let nums = Vec[10, 20, 30]
        sum_vec(nums)
        "#,
        60,
    );
}

#[test]
fn test_for_each_vec_empty() {
    // empty vec: loop body should not execute
    assert_aelys_int(
        r#"
        let v = Vec[]
        let mut count = 0
        for item in v {
            count++
        }
        count
        "#,
        0,
    );
}

#[test]
fn test_for_each_vec_break() {
    assert_aelys_int(
        r#"
        let v = Vec[1, 2, 3, 4, 5]
        let mut sum = 0
        for item in v {
            if item == 3 { break }
            sum += item
        }
        sum
        "#,
        3, // 1 + 2
    );
}

#[test]
fn test_for_each_vec_continue() {
    assert_aelys_int(
        r#"
        let v = Vec[1, 2, 3, 4, 5]
        let mut sum = 0
        for item in v {
            if item == 3 { continue }
            sum += item
        }
        sum
        "#,
        12, // 1 + 2 + 4 + 5
    );
}

#[test]
fn test_for_each_array_int() {
    assert_aelys_int(
        r#"
        let arr = [10, 20, 30]
        let mut sum = 0
        for item in arr {
            sum += item
        }
        sum
        "#,
        60,
    );
}

#[test]
fn test_for_each_array_empty() {
    assert_aelys_int(
        r#"
        let arr = []
        let mut count = 0
        for item in arr {
            count++
        }
        count
        "#,
        0,
    );
}

#[test]
fn test_for_each_array_bool() {
    // count true values
    assert_aelys_int(
        r#"
        let arr = [true, false, true, true, false]
        let mut count = 0
        for item in arr {
            if item { count++ }
        }
        count
        "#,
        3,
    );
}

#[test]
fn test_for_each_array_break() {
    assert_aelys_int(
        r#"
        let arr = [1, 2, 3, 4, 5]
        let mut sum = 0
        for item in arr {
            if item > 3 { break }
            sum += item
        }
        sum
        "#,
        6, // 1 + 2 + 3
    );
}

#[test]
fn test_for_each_array_float() {
    let result = run_aelys(
        r#"
        let arr = [1.5, 2.5, 3.0]
        let mut sum = 0.0
        for x in arr {
            sum += x
        }
        sum
        "#,
    );
    assert_eq!(result.as_float(), Some(7.0));
}

#[test]
fn test_for_each_nested_vec() {
    assert_aelys_int(
        r#"
        let rows = Vec[Vec[1, 2], Vec[3, 4], Vec[5, 6]]
        let mut sum = 0
        for row in rows {
            for item in row {
                sum += item
            }
        }
        sum
        "#,
        21,
    );
}

#[test]
fn test_string_method_len() {
    assert_aelys_int(r#""hello".len()"#, 5);
}

#[test]
fn test_string_method_trim() {
    assert_aelys_str(r#""  hi  ".trim()"#, "hi");
}

#[test]
fn test_string_method_contains() {
    assert_aelys_bool(r#""hello world".contains("world")"#, true);
    assert_aelys_bool(r#""hello".contains("xyz")"#, false);
}

#[test]
fn test_string_method_to_upper() {
    assert_aelys_str(r#""hello".to_upper()"#, "HELLO");
}

#[test]
fn test_string_method_to_lower() {
    assert_aelys_str(r#""HELLO".to_lower()"#, "hello");
}

#[test]
fn test_string_method_starts_with() {
    assert_aelys_bool(r#""hello world".starts_with("hello")"#, true);
    assert_aelys_bool(r#""hello world".starts_with("world")"#, false);
}

#[test]
fn test_string_method_ends_with() {
    assert_aelys_bool(r#""hello world".ends_with("world")"#, true);
    assert_aelys_bool(r#""hello world".ends_with("hello")"#, false);
}

#[test]
fn test_string_method_replace() {
    assert_aelys_str(r#""aaa".replace("a", "b")"#, "bbb");
}

#[test]
fn test_string_method_is_empty() {
    assert_aelys_bool(r#""".is_empty()"#, true);
    assert_aelys_bool(r#""hello".is_empty()"#, false);
}

#[test]
fn test_string_method_repeat() {
    assert_aelys_str(r#""ab".repeat(3)"#, "ababab");
}

#[test]
fn test_string_method_capitalize() {
    assert_aelys_str(r#""hello".capitalize()"#, "Hello");
}

#[test]
fn test_string_method_reverse() {
    assert_aelys_str(r#""abc".reverse()"#, "cba");
}

#[test]
fn test_string_method_trim_start() {
    assert_aelys_str(r#""  hi  ".trim_start()"#, "hi  ");
}

#[test]
fn test_string_method_trim_end() {
    assert_aelys_str(r#""  hi  ".trim_end()"#, "  hi");
}

#[test]
fn test_string_method_find() {
    assert_aelys_int(r#""hello world".find("world")"#, 6);
    assert_aelys_int(r#""hello".find("xyz")"#, -1);
}

#[test]
fn test_string_method_count() {
    assert_aelys_int(r#""banana".count("a")"#, 3);
}

#[test]
fn test_string_method_char_len() {
    assert_aelys_int(r#""hello".char_len()"#, 5);
}

#[test]
fn test_string_method_is_numeric() {
    assert_aelys_bool(r#""123".is_numeric()"#, true);
    assert_aelys_bool(r#""12a".is_numeric()"#, false);
}

#[test]
fn test_string_method_is_alphabetic() {
    assert_aelys_bool(r#""abc".is_alphabetic()"#, true);
    assert_aelys_bool(r#""a1c".is_alphabetic()"#, false);
}

#[test]
fn test_string_method_on_variable() {
    assert_aelys_str(
        r#"
        let s = "  hello  "
        s.trim()
        "#,
        "hello",
    );
}

#[test]
fn test_string_method_chaining() {
    // Chain: trim then to_upper
    assert_aelys_str(
        r#"
        let s = "  hello  "
        let trimmed = s.trim()
        trimmed.to_upper()
        "#,
        "HELLO",
    );
}

#[test]
fn test_to_string_int() {
    assert_aelys_str(r#"let x = 42; x.to_string()"#, "42");
}

#[test]
fn test_to_string_bool() {
    assert_aelys_str(r#"let x = true; x.to_string()"#, "true");
}

#[test]
fn test_to_string_null() {
    assert_aelys_str("let x = null; x.to_string()", "null");
}

mod common;

use common::{assert_aelys_int, run_aelys};

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

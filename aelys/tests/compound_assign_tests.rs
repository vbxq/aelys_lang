//! Tests for compound assignment (+=, -=, *=, /=, %=) and increment/decrement (++, --)

mod common;
use common::*;

// === Compound assignment on variables ===

#[test]
fn test_plus_eq() {
    assert_aelys_int(
        r#"
let mut x = 10
x += 5
x
"#,
        15,
    );
}

#[test]
fn test_minus_eq() {
    assert_aelys_int(
        r#"
let mut x = 10
x -= 3
x
"#,
        7,
    );
}

#[test]
fn test_star_eq() {
    assert_aelys_int(
        r#"
let mut x = 6
x *= 7
x
"#,
        42,
    );
}

#[test]
fn test_slash_eq() {
    assert_aelys_int(
        r#"
let mut x = 20
x /= 4
x
"#,
        5,
    );
}

#[test]
fn test_percent_eq() {
    assert_aelys_int(
        r#"
let mut x = 17
x %= 5
x
"#,
        2,
    );
}

// === Increment / Decrement ===

#[test]
fn test_increment() {
    assert_aelys_int(
        r#"
let mut count = 0
count++
count++
count++
count
"#,
        3,
    );
}

#[test]
fn test_decrement() {
    assert_aelys_int(
        r#"
let mut n = 10
n--
n--
n
"#,
        8,
    );
}

// === Compound assignment in loops ===

#[test]
fn test_plus_eq_in_for_loop() {
    assert_aelys_int(
        r#"
let mut sum = 0
for i in 0..10 {
    sum += i
}
sum
"#,
        45,
    );
}

#[test]
fn test_star_eq_in_loop() {
    assert_aelys_int(
        r#"
let mut product = 1
for i in 1..=5 {
    product *= i
}
product
"#,
        120,
    );
}

#[test]
fn test_increment_in_while_loop() {
    assert_aelys_int(
        r#"
let mut count = 0
while count < 10 {
    count++
}
count
"#,
        10,
    );
}

// === Compound assignment on array/vec index ===

#[test]
fn test_plus_eq_on_array_index() {
    assert_aelys_int(
        r#"
let arr = Array[10, 20, 30]
arr[1] += 5
arr[1]
"#,
        25,
    );
}

#[test]
fn test_minus_eq_on_array_index() {
    assert_aelys_int(
        r#"
let arr = Array[100, 200, 300]
arr[2] -= 50
arr[2]
"#,
        250,
    );
}

// === Compound assignment with floats ===

#[test]
fn test_plus_eq_float() {
    let result = run_aelys(
        r#"
let mut x = 1.5
x += 2.5
x
"#,
    );
    assert_eq!(result.as_float(), Some(4.0));
}

// === String concatenation with += ===

#[test]
fn test_plus_eq_string() {
    assert_aelys_str(
        r#"
let mut s = "hello"
s += " world"
s
"#,
        "hello world",
    );
}

// === Chained compound assignments ===

#[test]
fn test_all_compound_ops_chained() {
    assert_aelys_int(
        r#"
let mut x = 10
x += 5    // 15
x -= 3    // 12
x *= 2    // 24
x /= 4   // 6
x %= 5   // 1
x
"#,
        1,
    );
}

// === Compound assignment with expressions ===

#[test]
fn test_plus_eq_with_expression() {
    assert_aelys_int(
        r#"
let mut x = 10
let y = 3
x += y * 2
x
"#,
        16,
    );
}

// === Double negation still works ===

#[test]
fn test_double_negation_preserved() {
    assert_aelys_int("--42", 42);
}

// === Increment in function ===

#[test]
fn test_increment_in_function() {
    assert_aelys_int(
        r#"
fn count_up(n) {
    let mut count = 0
    for i in 0..n {
        count++
    }
    return count
}
count_up(100)
"#,
        100,
    );
}

// === Compound assign with mut param ===

#[test]
fn test_compound_assign_mut_param() {
    assert_aelys_int(
        r#"
fn accumulate(mut acc: int, n: int) -> int {
    for i in 0..n {
        acc += i
    }
    return acc
}
accumulate(10, 5)
"#,
        20,
    );
}

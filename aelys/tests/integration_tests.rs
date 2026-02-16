//! Integration tests for Aelys - converted from examples/test_*.aelys

mod common;
use common::*;

// ==Simple function tests =====

#[test]
fn test_simple_function_return() {
    let result = run_aelys(
        r#"
fn get_answer() {
    return 42
}
get_answer()
"#,
    );
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_function_with_params() {
    let result = run_aelys(
        r#"
fn add(a, b) {
    return a + b
}
add(10, 20)
"#,
    );
    assert_eq!(result.as_int(), Some(30));
}

// ==If expression tests =====

#[test]
fn test_if_expression_simple() {
    let result = run_aelys(
        r#"
let x = if true { 1 } else { 2 }
x
"#,
    );
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn test_if_expression_false() {
    let result = run_aelys(
        r#"
let x = if false { 1 } else { 2 }
x
"#,
    );
    assert_eq!(result.as_int(), Some(2));
}

#[test]
fn test_if_expression_with_return() {
    let result = run_aelys(
        r#"
fn test_return(n) {
    return if n > 0 { n } else { 0 }
}
test_return(5)
"#,
    );
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_if_expression_implicit_return() {
    let result = run_aelys(
        r#"
fn implicit_return(n) {
    if n > 0 { n } else { 0 }
}
implicit_return(5)
"#,
    );
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_nested_if_expression() {
    let result = run_aelys(
        r#"
fn nested(a, b) {
    if a > 0 {
        if b > 0 { a + b } else { a }
    } else {
        if b > 0 { b } else { 0 }
    }
}
nested(1, 2)
"#,
    );
    assert_eq!(result.as_int(), Some(3));
}

// ==Large integer tests =====

#[test]
fn test_large_integers() {
    // Integers exceeding 48-bit range should produce an error
    let err = run_aelys_err(
        r#"
let a = 9007199254740000
a
"#,
    );
    assert!(err.contains("integer") && err.contains("exceeds") && err.contains("range"));
}

#[test]
fn test_negative_large_integer() {
    // Negative integers exceeding 48-bit range should produce an error
    let err = run_aelys_err(
        r#"
let c = -9007199254740991
c
"#,
    );
    assert!(err.contains("integer") && err.contains("exceeds") && err.contains("range"));
}

#[test]
fn test_power_of_two_30() {
    let result = run_aelys("1073741824");
    assert_eq!(result.as_int(), Some(1073741824));
}

#[test]
fn test_power_of_two_40() {
    let result = run_aelys("1099511627776");
    assert_eq!(result.as_int(), Some(1099511627776));
}

// ==Recursion tests =====

#[test]
fn test_recursion_factorial() {
    let result = run_aelys(
        r#"
fn factorial(n) {
    if n <= 1 {
        return 1
    }
    return n * factorial(n - 1)
}
factorial(10)
"#,
    );
    assert_eq!(result.as_int(), Some(3628800));
}

#[test]
fn test_recursion_fibonacci() {
    let result = run_aelys(
        r#"
fn fib(n) {
    if n <= 1 {
        return n
    }
    return fib(n - 1) + fib(n - 2)
}
fib(10)
"#,
    );
    assert_eq!(result.as_int(), Some(55));
}

#[test]
fn test_deep_recursion_500() {
    let result = run_aelys(
        r#"
fn deep(n) {
    if n > 0 {
        return deep(n - 1)
    }
    return n
}
deep(500)
"#,
    );
    assert_eq!(result.as_int(), Some(0));
}

#[test]
fn test_deep_recursion_1020() {
    let result = run_aelys(
        r#"
fn deep(n) {
    if n > 0 {
        return deep(n - 1)
    }
    return n
}
deep(1020)
"#,
    );
    assert_eq!(result.as_int(), Some(0));
}

// ==Mutual recursion =====

#[test]
fn test_mutual_recursion_is_even() {
    let result = run_aelys(
        r#"
fn is_even(n) {
    if n == 0 {
        return true
    }
    return is_odd(n - 1)
}

fn is_odd(n) {
    if n == 0 {
        return false
    }
    return is_even(n - 1)
}

is_even(10)
"#,
    );
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_mutual_recursion_is_odd() {
    let result = run_aelys(
        r#"
fn is_even(n) {
    if n == 0 {
        return true
    }
    return is_odd(n - 1)
}

fn is_odd(n) {
    if n == 0 {
        return false
    }
    return is_even(n - 1)
}

is_odd(99)
"#,
    );
    assert_eq!(result.as_bool(), Some(true));
}

// ==Division tests =====

#[test]
fn test_integer_division() {
    let result = run_aelys("10 / 3");
    assert_eq!(result.as_int(), Some(3));
}

#[test]
fn test_exact_division() {
    let result = run_aelys("10 / 2");
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_negative_dividend() {
    let result = run_aelys("-10 / 3");
    assert_eq!(result.as_int(), Some(-3));
}

#[test]
fn test_negative_divisor() {
    let result = run_aelys("10 / -3");
    assert_eq!(result.as_int(), Some(-3));
}

#[test]
fn test_both_negative() {
    let result = run_aelys("-10 / -3");
    assert_eq!(result.as_int(), Some(3));
}

#[test]
fn test_float_division() {
    let result = run_aelys("10.0 / 3.0");
    let f = result.as_float().unwrap();
    assert!((f - 3.3333333333333335).abs() < 0.0001);
}

// ==Modulo tests =====

#[test]
fn test_modulo() {
    let result = run_aelys("17 % 5");
    assert_eq!(result.as_int(), Some(2));
}

// ==Error tests =====

#[test]
fn test_division_by_zero() {
    let result = aelys::run("10 / 0", "<test>");
    assert!(result.is_err());
}

#[test]
fn test_modulo_by_zero() {
    let result = aelys::run("10 % 0", "<test>");
    assert!(result.is_err());
}

// ==Logic tests =====

#[test]
fn test_and_short_circuit() {
    // false and X should return false without evaluating X
    let result = run_aelys(
        r#"
let r = false and true
r
"#,
    );
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_or_short_circuit() {
    // true or X should return true without evaluating X
    let result = run_aelys(
        r#"
let r = true or false
r
"#,
    );
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_complex_boolean() {
    let result = run_aelys("true and false or true");
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_grouped_boolean() {
    let result = run_aelys("(true or false) and (false or true)");
    assert_eq!(result.as_bool(), Some(true));
}

// ==Control flow tests =====

#[test]
fn test_while_loop() {
    let result = run_aelys(
        r#"
let mut i = 0
while i < 10 {
    i++
}
i
"#,
    );
    assert_eq!(result.as_int(), Some(10));
}

#[test]
fn test_while_with_break() {
    let result = run_aelys(
        r#"
let mut i = 0
while true {
    i++
    if i >= 5 {
        break
    }
}
i
"#,
    );
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_while_with_continue() {
    let result = run_aelys(
        r#"
let mut i = 0
let mut sum = 0
while i < 10 {
    i++
    if i % 2 == 0 {
        continue
    }
    sum += i
}
sum
"#,
    );
    // sum of odd numbers 1,3,5,7,9 = 25
    assert_eq!(result.as_int(), Some(25));
}

// ==Variable shadowing tests =====

#[test]
fn test_variable_shadowing_simple() {
    // Blocks don't return implicit values in Aelys
    // The result is null from the block statement
    let result = run_aelys(
        r#"
let x = 1
{
    let x = 2
}
x
"#,
    );
    // After the block, x is back to outer scope value
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn test_variable_shadowing_outer_restored() {
    let result = run_aelys(
        r#"
let x = 1
{
    let x = 2
}
x
"#,
    );
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn test_deep_shadowing() {
    // Test that deeply nested shadowing works correctly
    let result = run_aelys(
        r#"
fn get_inner() {
    let y = 0
    {
        let y = 1
        {
            let y = 2
            {
                let y = 3
                {
                    let y = 4
                    {
                        let y = 5
                        return y
                    }
                }
            }
        }
    }
}
get_inner()
"#,
    );
    assert_eq!(result.as_int(), Some(5));
}

// ==GC stress tests =====

#[test]
fn test_gc_stress_loop() {
    // This should complete without running out of memory
    let result = run_aelys(
        r#"
fn gc_stress() {
    let mut i = 0
    while i < 10000 {
        let temp = "garbage string that should be collected"
        i++
    }
    i
}
gc_stress()
"#,
    );
    assert_eq!(result.as_int(), Some(10000));
}

#[test]
fn test_gc_stress_concatenation() {
    // String concatenation stress test
    let result = run_aelys(
        r#"
fn gc_stress2() {
    let mut i = 0
    while i < 1000 {
        let a = "string a"
        let b = "string b"
        let c = a + b
        i++
    }
    i
}
gc_stress2()
"#,
    );
    assert_eq!(result.as_int(), Some(1000));
}

// ==Edge case tests =====

#[test]
fn test_negative_zero() {
    let result = run_aelys("-0");
    assert_eq!(result.as_int(), Some(0));
}

#[test]
fn test_double_negation() {
    let result = run_aelys("--42");
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_not_not() {
    let result = run_aelys("not not true");
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_comparison_chain() {
    // a < b should return bool, not chainable like Python
    let result = run_aelys("1 < 2");
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_equality_types() {
    // Different types should not be equal
    let result = run_aelys("42 == true");
    assert_eq!(result.as_bool(), Some(false));
}

// ==@no_gc tests =====

#[test]
fn test_no_gc_decorator() {
    let result = run_aelys(
        r#"
@no_gc
fn critical_section(n) {
    let mut sum = 0
    let mut i = 0
    while i < n {
        sum += i
        i++
    }
    return sum
}

critical_section(100)
"#,
    );
    // sum of 0..99 = 99*100/2 = 4950
    assert_eq!(result.as_int(), Some(4950));
}

#[test]
fn test_string_concatenation() {
    let result = run_aelys(
        r#"
let a = "Hello"
let b = " World"
a + b
"#,
    );
    // Since we can't easily check string content, just verify it runs without error
    assert!(result.is_ptr());
}

#[test]
fn test_precedence_mul_add() {
    let result = run_aelys("2 + 3 * 4");
    assert_eq!(result.as_int(), Some(14));
}

#[test]
fn test_precedence_grouped() {
    let result = run_aelys("(2 + 3) * 4");
    assert_eq!(result.as_int(), Some(20));
}

#[test]
fn test_complex_expression() {
    let result = run_aelys("1 + 2 * 3 - 4 / 2");
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_max_int_value() {
    // Test actual 48-bit max value: (1 << 47) - 1 = 140737488355327
    let result = run_aelys("140737488355327");
    assert_eq!(result.as_int(), Some(140737488355327));

    // Values exceeding 48-bit range should error
    let err = run_aelys_err("140737488355328");
    assert!(err.contains("integer") && err.contains("exceeds") && err.contains("range"));
}

#[test]
fn test_min_int_value() {
    // Test value near 48-bit min: -(1 << 47) = -140737488355328
    // We can't test -140737488355328 directly because the parser treats it as
    // unary negation of 140737488355328, which is already out of range.
    // So we test -140737488355327 instead (which is INT_MIN + 1)
    let result = run_aelys("-140737488355327");
    assert_eq!(result.as_int(), Some(-140737488355327));

    // Values exceeding 48-bit range should error
    // Test with a value whose absolute value exceeds INT_MAX
    let err = run_aelys_err("-140737488355328");
    assert!(err.contains("integer") && err.contains("exceeds") && err.contains("range"));
}

#[test]
fn test_block_as_statement() {
    // Blocks are statements in Aelys, not expressions
    // They don't return values - use functions for that
    let result = run_aelys(
        r#"
fn compute() {
    let a = 10
    let b = 20
    a + b
}
compute()
"#,
    );
    assert_eq!(result.as_int(), Some(30));
}

#[test]
fn test_many_variables() {
    let result = run_aelys(
        r#"
let a1 = 1
let a2 = 2
let a3 = 3
let a4 = 4
let a5 = 5
let a6 = 6
let a7 = 7
let a8 = 8
let a9 = 9
let a10 = 10
a1 + a2 + a3 + a4 + a5 + a6 + a7 + a8 + a9 + a10
"#,
    );
    assert_eq!(result.as_int(), Some(55));
}

#[test]
fn test_nested_function_calls() {
    let result = run_aelys(
        r#"
fn add(a, b) { a + b }
fn mul(a, b) { a * b }
fn combined(a, b, c) {
    add(mul(a, b), c)
}
combined(2, 3, 4)
"#,
    );
    assert_eq!(result.as_int(), Some(10));
}

#[test]
fn test_empty_function() {
    let result = run_aelys(
        r#"
fn empty() { }
empty()
"#,
    );
    assert!(result.is_null());
}

#[test]
fn test_early_return() {
    let result = run_aelys(
        r#"
fn early(n) {
    if n < 0 {
        return -1
    }
    if n == 0 {
        return 0
    }
    return 1
}
early(-5)
"#,
    );
    assert_eq!(result.as_int(), Some(-1));
}

#[test]
fn test_early_return_zero() {
    let result = run_aelys(
        r#"
fn early(n) {
    if n < 0 {
        return -1
    }
    if n == 0 {
        return 0
    }
    return 1
}
early(0)
"#,
    );
    assert_eq!(result.as_int(), Some(0));
}

#[test]
fn test_early_return_positive() {
    let result = run_aelys(
        r#"
fn early(n) {
    if n < 0 {
        return -1
    }
    if n == 0 {
        return 0
    }
    return 1
}
early(5)
"#,
    );
    assert_eq!(result.as_int(), Some(1));
}

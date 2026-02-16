/// Integration tests for the Aelys language.
///
/// These tests verify end-to-end execution of Aelys programs,
/// running the full pipeline: Lexer -> Parser -> Compiler -> VM.
use aelys::run;
use aelys_runtime::Value;

/// Helper to run code and expect success
fn run_ok(source: &str) -> Value {
    run(source, "test.aelys").expect("Expected program to run successfully")
}

/// Helper to run code and expect an error
fn run_err(source: &str) -> String {
    match run(source, "test.aelys") {
        Ok(_) => panic!("Expected program to fail, but it succeeded"),
        Err(e) => format!("{}", e),
    }
}

#[test]
fn test_arithmetic() {
    // Simple arithmetic
    let result = run_ok("1 + 2;");
    assert_eq!(result.as_int(), Some(3));

    let result = run_ok("10 - 3;");
    assert_eq!(result.as_int(), Some(7));

    let result = run_ok("4 * 5;");
    assert_eq!(result.as_int(), Some(20));

    let result = run_ok("20 / 4;");
    assert_eq!(result.as_int(), Some(5));

    let result = run_ok("17 % 5;");
    assert_eq!(result.as_int(), Some(2));
}

#[test]
fn test_complex_arithmetic() {
    let result = run_ok("(1 + 2) * 3;");
    assert_eq!(result.as_int(), Some(9));

    let result = run_ok("10 - 2 * 3;");
    assert_eq!(result.as_int(), Some(4));

    let result = run_ok("(10 - 2) * 3;");
    assert_eq!(result.as_int(), Some(24));
}

#[test]
fn test_variables() {
    let result = run_ok("let x = 42; x;");
    assert_eq!(result.as_int(), Some(42));

    let result = run_ok("let x = 10; let y = 20; x + y;");
    assert_eq!(result.as_int(), Some(30));
}

#[test]
fn test_mutable_variables() {
    let result = run_ok("let mut x = 10; x = 20; x;");
    assert_eq!(result.as_int(), Some(20));

    let result = run_ok("let mut count = 0; count++; count;");
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn test_comparisons() {
    let result = run_ok("1 < 2;");
    assert_eq!(result.as_bool(), Some(true));

    let result = run_ok("2 < 1;");
    assert_eq!(result.as_bool(), Some(false));

    let result = run_ok("1 <= 1;");
    assert_eq!(result.as_bool(), Some(true));

    let result = run_ok("2 > 1;");
    assert_eq!(result.as_bool(), Some(true));

    let result = run_ok("1 == 1;");
    assert_eq!(result.as_bool(), Some(true));

    let result = run_ok("1 != 2;");
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_logical_operators() {
    let result = run_ok("true and true;");
    assert_eq!(result.as_bool(), Some(true));

    let result = run_ok("true and false;");
    assert_eq!(result.as_bool(), Some(false));

    let result = run_ok("false or true;");
    assert_eq!(result.as_bool(), Some(true));

    let result = run_ok("false or false;");
    assert_eq!(result.as_bool(), Some(false));

    let result = run_ok("not true;");
    assert_eq!(result.as_bool(), Some(false));

    let result = run_ok("not false;");
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_if_statement() {
    // if with else at top level returns the value of the taken branch
    let result = run_ok("if true { 42 } else { 0 }");
    assert_eq!(result.as_int(), Some(42));

    let result = run_ok("let x = if true { 42 } else { 0 }; x;");
    assert_eq!(result.as_int(), Some(42));

    let result = run_ok("let x = if false { 42 } else { 100 }; x;");
    assert_eq!(result.as_int(), Some(100));
}

#[test]
fn test_while_loop() {
    let result = run_ok(
        r#"
        let mut i = 0;
        let mut sum = 0;
        while i < 5 {
            sum += i;
            i++;
        }
        sum;
    "#,
    );
    assert_eq!(result.as_int(), Some(10)); // 0 + 1 + 2 + 3 + 4
}

#[test]
fn test_factorial_iterative() {
    let result = run_ok(
        r#"
        let mut n = 5;
        let mut result = 1;
        while n > 0 {
            result *= n;
            n--;
        }
        result;
    "#,
    );
    assert_eq!(result.as_int(), Some(120)); // 5! = 120
}

#[test]
fn test_fibonacci_recursive() {
    let result = run_ok(
        r#"
        fn fib(n) {
            if n < 2 {
                return n;
            }
            return fib(n - 1) + fib(n - 2);
        }
        fib(10);
    "#,
    );
    assert_eq!(result.as_int(), Some(55)); // fib(10) = 55
}

#[test]
fn test_factorial_recursive() {
    let result = run_ok(
        r#"
        fn factorial(n) {
            if n <= 1 {
                return 1;
            }
            return n * factorial(n - 1);
        }
        factorial(5);
    "#,
    );
    assert_eq!(result.as_int(), Some(120)); // 5! = 120
}

#[test]
fn test_fizzbuzz() {
    // FizzBuzz for numbers 1-15
    // We'll test by capturing the logic in variables
    let result = run_ok(
        r#"
        let mut i = 1;
        let mut count = 0;
        while i <= 15 {
            if i % 15 == 0 {
                count++;
            } else {
                if i % 3 == 0 {
                    count++;
                } else {
                    if i % 5 == 0 {
                        count++;
                    }
                }
            }
            i++;
        }
        count;
    "#,
    );
    assert_eq!(result.as_int(), Some(7)); // Numbers divisible by 3 or 5: 3, 5, 6, 9, 10, 12, 15
}

#[test]
fn test_higher_order_functions() {
    let result = run_ok(
        r#"
        fn apply_twice(f, x) {
            return f(f(x));
        }
        fn increment(n) {
            return n + 1;
        }
        apply_twice(increment, 10);
    "#,
    );
    assert_eq!(result.as_int(), Some(12)); // increment(increment(10)) = 12
}

#[test]
fn test_function_with_multiple_params() {
    let result = run_ok(
        r#"
        fn add(a, b) {
            return a + b;
        }
        fn multiply(a, b) {
            return a * b;
        }
        multiply(add(2, 3), 4);
    "#,
    );
    assert_eq!(result.as_int(), Some(20)); // (2 + 3) * 4 = 20
}

#[test]
fn test_variable_scoping() {
    // Test block scoping - inner x shadows outer x
    let result = run_ok(
        r#"
        let x = 1;
        {
            let x = 2;
            let y = x + 1;
        }
        x;
    "#,
    );
    assert_eq!(result.as_int(), Some(1)); // Outer x is unchanged

    // Test nested block scoping
    let result = run_ok(
        r#"
        let x = 5;
        {
            let x = 10;
            {
                let x = 15;
            }
        }
        x;
    "#,
    );
    assert_eq!(result.as_int(), Some(5)); // Outer x is still 5
}

#[test]
fn test_break_statement() {
    let result = run_ok(
        r#"
        let mut i = 0;
        let mut sum = 0;
        while i < 100 {
            if i == 5 {
                break;
            }
            sum += i;
            i++;
        }
        sum;
    "#,
    );
    assert_eq!(result.as_int(), Some(10)); // 0 + 1 + 2 + 3 + 4
}

#[test]
fn test_continue_statement() {
    let result = run_ok(
        r#"
        let mut i = 0;
        let mut sum = 0;
        while i < 5 {
            i++;
            if i == 3 {
                continue;
            }
            sum += i;
        }
        sum;
    "#,
    );
    assert_eq!(result.as_int(), Some(12)); // 1 + 2 + 4 + 5 (skips 3)
}

#[test]
fn test_nested_blocks() {
    let result = run_ok(
        r#"
        let x = 1;
        {
            let y = 2;
            {
                let z = 3;
                x + y + z;
            }
        }
        x;
    "#,
    );
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn test_error_undefined_variable() {
    let error = run_err("x;");
    assert!(error.contains("undefined variable"));
}

#[test]
fn test_error_assign_to_immutable() {
    let error = run_err("let x = 1; x = 2;");
    assert!(error.contains("immutable") || error.contains("not mutable"));
}

#[test]
fn test_error_assign_to_immutable_param() {
    let error = run_err("fn f(x: int) -> int { x++; x } f(1);");
    assert!(error.contains("immutable") || error.contains("not mutable"));
}

#[test]
fn test_mut_param_reassign() {
    let result = run_ok("fn f(mut x: int) -> int { x++; x } f(10);");
    assert_eq!(result.as_int(), Some(11));
}

#[test]
fn test_mut_param_in_loop() {
    let result = run_ok(
        "fn accumulate(mut acc: int, n: int) -> int { \
         for i in 0..n { acc++ } \
         return acc } \
         accumulate(10, 5);",
    );
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_mut_param_does_not_affect_caller() {
    let result = run_ok(
        "fn inc(mut x: int) -> int { x += 100; x } \
         let a = 1; let b = inc(a); a;",
    );
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn test_error_break_outside_loop() {
    let error = run_err("break;");
    assert!(error.contains("break") && error.contains("loop"));
}

#[test]
fn test_error_continue_outside_loop() {
    let error = run_err("continue;");
    assert!(error.contains("continue") && error.contains("loop"));
}

#[test]
fn test_error_division_by_zero() {
    let error = run_err("1 / 0;");
    assert!(error.contains("division by zero"));
}

#[test]
fn test_error_arity_mismatch() {
    let error = run_err(
        r#"
        fn add(a, b) {
            return a + b;
        }
        add(1);
    "#,
    );
    assert!(error.contains("arity") || error.contains("arguments"));
}

#[test]
fn test_empty_program() {
    let result = run_ok("");
    assert!(result.is_null());
}

#[test]
fn test_expression_only() {
    let result = run_ok("42;");
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_boolean_literals() {
    let result = run_ok("true;");
    assert_eq!(result.as_bool(), Some(true));

    let result = run_ok("false;");
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_null_literal() {
    let result = run_ok("null;");
    assert!(result.is_null());
}

#[test]
fn test_negation() {
    let result = run_ok("-42;");
    assert_eq!(result.as_int(), Some(-42));

    let result = run_ok("-(10 - 5);");
    assert_eq!(result.as_int(), Some(-5));
}

#[test]
fn test_multiple_statements() {
    let result = run_ok(
        r#"
        let a = 1;
        let b = 2;
        let c = 3;
        a + b + c;
    "#,
    );
    assert_eq!(result.as_int(), Some(6));
}

#[test]
fn test_return_from_main() {
    // The last expression/statement determines the program's return value
    let result = run_ok("42;");
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_function_returns_early() {
    let result = run_ok(
        r#"
        fn test(x) {
            if x < 0 {
                return 0;
            }
            return x * 2;
        }
        test(-5);
    "#,
    );
    assert_eq!(result.as_int(), Some(0));

    let result = run_ok(
        r#"
        fn test(x) {
            if x < 0 {
                return 0;
            }
            return x * 2;
        }
        test(5);
    "#,
    );
    assert_eq!(result.as_int(), Some(10));
}

#[test]
fn test_nested_function_calls() {
    let result = run_ok(
        r#"
        fn double(x) {
            return x * 2;
        }
        fn quadruple(x) {
            return double(double(x));
        }
        quadruple(5);
    "#,
    );
    assert_eq!(result.as_int(), Some(20));
}

#[test]
fn test_function_with_no_params() {
    let result = run_ok(
        r#"
        fn get_answer() {
            return 42;
        }
        get_answer();
    "#,
    );
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_function_no_explicit_return() {
    let result = run_ok(
        r#"
        fn implicit_return() {
            let x = 10;
        }
        implicit_return();
    "#,
    );
    assert!(result.is_null());
}

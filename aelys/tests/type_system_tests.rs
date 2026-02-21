use aelys::run;
use aelys_runtime::Value;

/// Helper to run code and expect success
fn run_ok(source: &str) -> Value {
    run(source, "test.aelys").expect("Expected program to run successfully")
}

/// Helper to run code and expect an error
#[allow(dead_code)]
fn run_err(source: &str) -> String {
    run(source, "test.aelys")
        .expect_err("Expected program to fail")
        .to_string()
}

#[test]
fn test_let_with_int_type() {
    let result = run_ok(
        r#"
        let x: int = 42
        x
    "#,
    );
    assert_eq!(result.as_int(), Some(42));
}

#[test]
#[allow(clippy::approx_constant)]
fn test_let_with_float_type() {
    let result = run_ok(
        r#"
        let x: float = 3.14
        x
    "#,
    );
    assert!((result.as_float().unwrap() - 3.14).abs() < 0.001);
}

#[test]
fn test_let_with_bool_type() {
    let result = run_ok(
        r#"
        let x: bool = true
        x
    "#,
    );
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_let_with_string_type() {
    let result = run_ok(
        r#"
        let x: string = "hello"
        x
    "#,
    );
    // String comparison would need heap access, just verify it runs
    assert!(result.as_ptr().is_some());
}

#[test]
fn test_let_mutable_with_type() {
    let result = run_ok(
        r#"
        let mut x: int = 10
        x += 5
        x
    "#,
    );
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_function_with_typed_params() {
    let result = run_ok(
        r#"
        fn add(a: int, b: int) {
            return a + b
        }
        add(3, 4)
    "#,
    );
    assert_eq!(result.as_int(), Some(7));
}

#[test]
fn test_function_with_return_type() {
    let result = run_ok(
        r#"
        fn square(x: int) -> int {
            return x * x
        }
        square(5)
    "#,
    );
    assert_eq!(result.as_int(), Some(25));
}

#[test]
fn test_function_with_typed_params_and_return() {
    let result = run_ok(
        r#"
        fn multiply(a: int, b: int) -> int {
            return a * b
        }
        multiply(6, 7)
    "#,
    );
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_function_void_return_type() {
    let result = run_ok(
        r#"
        let mut counter: int = 0

        fn increment() -> void {
            counter++
        }

        increment()
        increment()
        counter
    "#,
    );
    assert_eq!(result.as_int(), Some(2));
}

#[test]
fn test_lambda_with_typed_params() {
    let result = run_ok(
        r#"
        let add = fn(a: int, b: int) { return a + b }
        add(10, 20)
    "#,
    );
    assert_eq!(result.as_int(), Some(30));
}

#[test]
fn test_lambda_with_return_type() {
    let result = run_ok(
        r#"
        let double = fn(x: int) -> int { return x * 2 }
        double(15)
    "#,
    );
    assert_eq!(result.as_int(), Some(30));
}

#[test]
fn test_infer_int_literal() {
    let result = run_ok(
        r#"
        let x = 100
        x + 50
    "#,
    );
    assert_eq!(result.as_int(), Some(150));
}

#[test]
fn test_infer_float_literal() {
    let result = run_ok(
        r#"
        let x = 2.5
        x * 4.0
    "#,
    );
    assert!((result.as_float().unwrap() - 10.0).abs() < 0.001);
}

#[test]
fn test_infer_bool_literal() {
    let result = run_ok(
        r#"
        let x = true
        let y = false
        x and not y
    "#,
    );
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_infer_from_binary_op() {
    let result = run_ok(
        r#"
        let sum = 10 + 20
        let diff = sum - 5
        diff
    "#,
    );
    assert_eq!(result.as_int(), Some(25));
}

#[test]
fn test_int_addition_specialized() {
    // This should emit AddII opcode
    let result = run_ok(
        r#"
        let a: int = 100
        let b: int = 200
        a + b
    "#,
    );
    assert_eq!(result.as_int(), Some(300));
}

#[test]
fn test_int_subtraction_specialized() {
    // This should emit SubII opcode
    let result = run_ok(
        r#"
        let a: int = 500
        let b: int = 123
        a - b
    "#,
    );
    assert_eq!(result.as_int(), Some(377));
}

#[test]
fn test_int_multiplication_specialized() {
    // This should emit MulII opcode
    let result = run_ok(
        r#"
        let a: int = 12
        let b: int = 11
        a * b
    "#,
    );
    assert_eq!(result.as_int(), Some(132));
}

#[test]
fn test_int_division_specialized() {
    // This should emit DivII opcode
    let result = run_ok(
        r#"
        let a: int = 100
        let b: int = 4
        a / b
    "#,
    );
    assert_eq!(result.as_int(), Some(25));
}

#[test]
fn test_int_modulo_specialized() {
    // This should emit ModII opcode
    let result = run_ok(
        r#"
        let a: int = 17
        let b: int = 5
        a % b
    "#,
    );
    assert_eq!(result.as_int(), Some(2));
}

#[test]
fn test_int_comparison_lt_specialized() {
    // This should emit LtII opcode
    let result = run_ok(
        r#"
        let a: int = 10
        let b: int = 20
        a < b
    "#,
    );
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_int_comparison_le_specialized() {
    // This should emit LeII opcode
    let result = run_ok(
        r#"
        let a: int = 10
        let b: int = 10
        a <= b
    "#,
    );
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_int_comparison_gt_specialized() {
    // This should emit GtII opcode
    let result = run_ok(
        r#"
        let a: int = 30
        let b: int = 20
        a > b
    "#,
    );
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_int_comparison_ge_specialized() {
    // This should emit GeII opcode
    let result = run_ok(
        r#"
        let a: int = 20
        let b: int = 20
        a >= b
    "#,
    );
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_int_equality_specialized() {
    // This should emit EqII opcode
    let result = run_ok(
        r#"
        let a: int = 42
        let b: int = 42
        a == b
    "#,
    );
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_int_not_equal_specialized() {
    // This should emit NeII opcode
    let result = run_ok(
        r#"
        let a: int = 42
        let b: int = 43
        a != b
    "#,
    );
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_float_addition_specialized() {
    // This should emit AddFF opcode
    let result = run_ok(
        r#"
        let a: float = 1.5
        let b: float = 2.5
        a + b
    "#,
    );
    assert!((result.as_float().unwrap() - 4.0).abs() < 0.001);
}

#[test]
fn test_float_subtraction_specialized() {
    // This should emit SubFF opcode
    let result = run_ok(
        r#"
        let a: float = 10.0
        let b: float = 3.5
        a - b
    "#,
    );
    assert!((result.as_float().unwrap() - 6.5).abs() < 0.001);
}

#[test]
fn test_float_multiplication_specialized() {
    // This should emit MulFF opcode
    let result = run_ok(
        r#"
        let a: float = 2.5
        let b: float = 4.0
        a * b
    "#,
    );
    assert!((result.as_float().unwrap() - 10.0).abs() < 0.001);
}

#[test]
fn test_float_division_specialized() {
    // This should emit DivFF opcode
    let result = run_ok(
        r#"
        let a: float = 15.0
        let b: float = 3.0
        a / b
    "#,
    );
    assert!((result.as_float().unwrap() - 5.0).abs() < 0.001);
}

#[test]
fn test_int_float_mixed_arithmetic() {
    // When types are mixed, the generic opcodes should be used
    let result = run_ok(
        r#"
        let a: int = 5
        let b: float = 2.5
        a + b
    "#,
    );
    assert!((result.as_float().unwrap() - 7.5).abs() < 0.001);
}

#[test]
fn test_inferred_types_in_loop() {
    // Loop counter should be inferred as int
    let result = run_ok(
        r#"
        let mut sum = 0
        let mut i = 0
        while i < 10 {
            sum += i
            i++
        }
        sum
    "#,
    );
    assert_eq!(result.as_int(), Some(45));
}

#[test]
fn test_typed_loop_counter() {
    let result = run_ok(
        r#"
        let mut sum: int = 0
        let mut i: int = 0
        while i < 5 {
            sum += i * i
            i++
        }
        sum
    "#,
    );
    // 0 + 1 + 4 + 9 + 16 = 30
    assert_eq!(result.as_int(), Some(30));
}

#[test]
fn test_for_loop_with_typed_bounds() {
    let result = run_ok(
        r#"
        let mut sum: int = 0
        for i in 1..6 {
            sum += i
        }
        sum
    "#,
    );
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_nested_function_with_types() {
    let result = run_ok(
        r#"
        fn outer(x: int) -> int {
            fn inner(y: int) -> int {
                return y * 2
            }
            return inner(x) + 1
        }
        outer(10)
    "#,
    );
    assert_eq!(result.as_int(), Some(21));
}

#[test]
fn test_closure_with_typed_capture() {
    let result = run_ok(
        r#"
        fn make_adder(x: int) {
            return fn(y: int) -> int {
                return x + y
            }
        }
        let add10 = make_adder(10)
        add10(5)
    "#,
    );
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_recursive_function_with_types() {
    let result = run_ok(
        r#"
        fn factorial(n: int) -> int {
            if n <= 1 {
                return 1
            }
            return n * factorial(n - 1)
        }
        factorial(5)
    "#,
    );
    assert_eq!(result.as_int(), Some(120));
}

#[test]
fn test_multiple_typed_functions() {
    let result = run_ok(
        r#"
        fn square(x: int) -> int {
            return x * x
        }

        fn cube(x: int) -> int {
            return x * square(x)
        }

        cube(3)
    "#,
    );
    assert_eq!(result.as_int(), Some(27));
}

#[test]
fn test_various_int_types() {
    let result = run_ok(
        r#"
        let a: int = 10
        let b: int = 20
        let c: int = 30
        let d: int64 = 40
        a + b + c + d
    "#,
    );
    assert_eq!(result.as_int(), Some(100));
}

#[test]
fn test_various_uint_types() {
    let result = run_ok(
        r#"
        let a: int = 10
        let b: int = 20
        a + b
    "#,
    );
    assert_eq!(result.as_int(), Some(30));
}

#[test]
fn test_various_float_types() {
    let result = run_ok(
        r#"
        let a: float = 1.5
        let b: float64 = 2.5
        a + b
    "#,
    );
    assert!((result.as_float().unwrap() - 4.0).abs() < 0.001);
}

#[test]
fn test_function_without_type_annotations() {
    let result = run_ok(
        r#"
        fn add(a, b) {
            return a + b
        }
        add(10, 20)
    "#,
    );
    assert_eq!(result.as_int(), Some(30));
}

#[test]
fn test_let_without_type_annotation() {
    let result = run_ok(
        r#"
        let x = 42
        let y = x + 8
        y
    "#,
    );
    assert_eq!(result.as_int(), Some(50));
}

#[test]
fn test_lambda_without_type_annotations() {
    let result = run_ok(
        r#"
        let double = fn(x) { return x * 2 }
        double(25)
    "#,
    );
    assert_eq!(result.as_int(), Some(50));
}

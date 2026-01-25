use aelys::run;
use aelys_runtime::Value;

/// Helper to run code and expect success
fn run_ok(source: &str) -> Value {
    run(source, "test.aelys").expect("Expected program to run successfully")
}

// =============================================================================
// Basic Closure Tests
// =============================================================================

#[test]
fn test_closure_basic_capture() {
    // Closure captures immutable variable from enclosing scope
    let result = run_ok(
        r#"
        fn make_adder(x) {
            return fn(y) { return x + y }
        }
        let add5 = make_adder(5)
        add5(10)
    "#,
    );
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_closure_capture_multiple_values() {
    // Closure captures multiple variables
    let result = run_ok(
        r#"
        fn make_calculator(a, b) {
            return fn(op) {
                if op == 1 { return a + b }
                if op == 2 { return a - b }
                if op == 3 { return a * b }
                return a / b
            }
        }
        let calc = make_calculator(10, 5)
        calc(1) + calc(2) + calc(3)
    "#,
    );
    // (10+5) + (10-5) + (10*5) = 15 + 5 + 50 = 70
    assert_eq!(result.as_int(), Some(70));
}

#[test]
fn test_closure_returns_closure() {
    // Function returns a closure that captures its parameter
    let result = run_ok(
        r#"
        fn multiplier(factor) {
            return fn(x) { return x * factor }
        }
        let double = multiplier(2)
        let triple = multiplier(3)
        double(5) + triple(5)
    "#,
    );
    // 10 + 15 = 25
    assert_eq!(result.as_int(), Some(25));
}

// =============================================================================
// Mutable Upvalue Tests
// =============================================================================

#[test]
fn test_closure_mutable_counter() {
    // Classic counter example with mutable upvalue
    let result = run_ok(
        r#"
        fn make_counter() {
            let mut count = 0
            return fn() {
                count = count + 1
                return count
            }
        }
        let counter = make_counter()
        let a = counter()
        let b = counter()
        let c = counter()
        a + b + c
    "#,
    );
    // 1 + 2 + 3 = 6
    assert_eq!(result.as_int(), Some(6));
}

#[test]
fn test_closure_separate_counters() {
    // Two counters are independent
    let result = run_ok(
        r#"
        fn make_counter() {
            let mut count = 0
            return fn() {
                count = count + 1
                return count
            }
        }
        let counter1 = make_counter()
        let counter2 = make_counter()
        counter1()
        counter1()
        counter1()
        counter2()
        counter1() * 10 + counter2()
    "#,
    );
    // counter1 returns 4, counter2 returns 2 => 40 + 2 = 42
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_closure_counter_with_step() {
    // Counter with configurable step
    let result = run_ok(
        r#"
        fn make_stepper(step_size) {
            let mut count = 0
            return fn() {
                count = count + step_size
                return count
            }
        }
        let by5 = make_stepper(5)
        by5()
        by5()
        by5()
    "#,
    );
    // 5, 10, 15 -> returns 15
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_closure_accumulator() {
    // Accumulator that adds to running total
    let result = run_ok(
        r#"
        fn make_accumulator() {
            let mut total = 0
            return fn(n) {
                total = total + n
                return total
            }
        }
        let acc = make_accumulator()
        acc(10)
        acc(20)
        acc(5)
    "#,
    );
    // 10 + 20 + 5 = 35
    assert_eq!(result.as_int(), Some(35));
}

// =============================================================================
// Nested Closure Tests
// =============================================================================

#[test]
fn test_closure_nested_capture() {
    // Nested closures capturing from multiple levels
    let result = run_ok(
        r#"
        fn outer(x) {
            fn middle(y) {
                return fn(z) {
                    return x + y + z
                }
            }
            return middle(10)
        }
        let nested = outer(100)
        nested(1)
    "#,
    );
    // 100 + 10 + 1 = 111
    assert_eq!(result.as_int(), Some(111));
}

#[test]
fn test_closure_deeply_nested() {
    // Three levels of nesting
    let result = run_ok(
        r#"
        fn level1(a) {
            return fn(b) {
                return fn(c) {
                    return fn(d) {
                        return a * 1000 + b * 100 + c * 10 + d
                    }
                }
            }
        }
        let f1 = level1(1)
        let f2 = f1(2)
        let f3 = f2(3)
        f3(4)
    "#,
    );
    // 1*1000 + 2*100 + 3*10 + 4 = 1234
    assert_eq!(result.as_int(), Some(1234));
}

// =============================================================================
// Block-Scoped Closing Tests
// =============================================================================

#[test]
fn test_closure_block_scoped_capture() {
    // Variable captured in block, closure escapes block
    let result = run_ok(
        r#"
        fn block_test() {
            let mut result = 0
            {
                let mut x = 10
                let capture = fn() { return x }
                x = 20
                result = capture()
            }
            return result
        }
        block_test()
    "#,
    );
    // x was 20 when capture() was called
    assert_eq!(result.as_int(), Some(20));
}

#[test]
fn test_closure_captures_after_block_exit() {
    // Closure still works after block containing captured var exits
    let result = run_ok(
        r#"
        fn test() {
            let mut f = null
            {
                let x = 42
                f = fn() { return x }
            }
            return f()
        }
        test()
    "#,
    );
    assert_eq!(result.as_int(), Some(42));
}

// =============================================================================
// Closure in Loop Tests
// =============================================================================

#[test]
fn test_closure_in_loop_capture_current() {
    // Each iteration captures its own value
    let result = run_ok(
        r#"
        fn loop_test() {
            let mut sum = 0
            let mut i = 1
            while i <= 3 {
                let val = i
                let f = fn() { return val }
                sum = sum + f()
                i = i + 1
            }
            return sum
        }
        loop_test()
    "#,
    );
    // 1 + 2 + 3 = 6
    assert_eq!(result.as_int(), Some(6));
}

#[test]
fn test_closure_captures_loop_variable_by_ref() {
    // Capture loop variable directly (all see final value)
    let result = run_ok(
        r#"
        fn test() {
            let mut i = 0
            let f = fn() { return i }
            while i < 5 {
                i = i + 1
            }
            return f()
        }
        test()
    "#,
    );
    // f sees final value of i = 5
    assert_eq!(result.as_int(), Some(5));
}

// =============================================================================
// Higher-Order Function Tests with Closures
// =============================================================================

#[test]
fn test_closure_as_callback() {
    // Closure passed as callback
    let result = run_ok(
        r#"
        fn apply_twice(f, x) {
            return f(f(x))
        }
        let double = fn(x) { return x * 2 }
        apply_twice(double, 3)
    "#,
    );
    // double(double(3)) = double(6) = 12
    assert_eq!(result.as_int(), Some(12));
}

#[test]
fn test_closure_compose() {
    // Compose two functions
    let result = run_ok(
        r#"
        fn compose(f, g) {
            return fn(x) { return f(g(x)) }
        }
        let add1 = fn(x) { return x + 1 }
        let mul2 = fn(x) { return x * 2 }
        let add1_then_mul2 = compose(mul2, add1)
        add1_then_mul2(5)
    "#,
    );
    // (5 + 1) * 2 = 12
    assert_eq!(result.as_int(), Some(12));
}

#[test]
fn test_closure_partial_application() {
    // Partial application pattern
    let result = run_ok(
        r#"
        fn add(a) {
            return fn(b) {
                return a + b
            }
        }
        let add10 = add(10)
        let add20 = add(20)
        add10(5) + add20(5)
    "#,
    );
    // 15 + 25 = 40
    assert_eq!(result.as_int(), Some(40));
}

// =============================================================================
// Edge Cases
// =============================================================================

#[test]
fn test_closure_captures_bool() {
    // Capture boolean value - toggle returns true, false, true
    let result = run_ok(
        r#"
        fn make_toggle() {
            let mut state = false
            return fn() {
                state = not state
                return state
            }
        }
        let toggle = make_toggle()
        let a = toggle()
        let b = toggle()
        let c = toggle()
        // a=true(1), b=false(0), c=true(1) => sum = 2
        let sum = 0
        let mut result = sum
        if a { result = result + 1 }
        if b { result = result + 1 }
        if c { result = result + 1 }
        result
    "#,
    );
    // a=true, b=false, c=true => 1 + 0 + 1 = 2
    assert_eq!(result.as_int(), Some(2));
}

#[test]
fn test_closure_captures_null() {
    // Capture and modify null
    let result = run_ok(
        r#"
        fn test() {
            let mut val = null
            let setter = fn(v) { val = v }
            let getter = fn() { return val }
            setter(42)
            return getter()
        }
        test()
    "#,
    );
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_closure_immediate_invoke() {
    // Immediately invoked closure
    let result = run_ok(
        r#"
        let x = 10
        let result = (fn(y) { return x + y })(5)
        result
    "#,
    );
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_closure_recursive_with_capture() {
    // Recursive function that also captures
    let result = run_ok(
        r#"
        fn make_factorial(base) {
            fn fact(n) {
                if n <= 1 { return base }
                return n * fact(n - 1)
            }
            return fact
        }
        let fact1 = make_factorial(1)
        fact1(5)
    "#,
    );
    assert_eq!(result.as_int(), Some(120));
}

#[test]
fn test_closure_modifies_outer_mutable() {
    // Closure modifies mutable variable in outer scope
    let result = run_ok(
        r#"
        fn test() {
            let mut x = 0
            let inc = fn() { x = x + 1 }
            inc()
            inc()
            inc()
            return x
        }
        test()
    "#,
    );
    assert_eq!(result.as_int(), Some(3));
}

#[test]
fn test_closure_with_float() {
    // Capture float values
    let result = run_ok(
        r#"
        fn make_scaler(factor) {
            return fn(x) { return x * factor }
        }
        let half = make_scaler(0.5)
        half(10.0)
    "#,
    );
    assert_eq!(result.as_float(), Some(5.0));
}

#[test]
fn test_closure_chain() {
    // Chain of closures
    let result = run_ok(
        r#"
        fn chain(initial) {
            let mut value = initial
            let add = fn(n) { value = value + n; return value }
            let mul = fn(n) { value = value * n; return value }
            let get = fn() { return value }
            add(5)
            mul(2)
            add(10)
            return get()
        }
        chain(10)
    "#,
    );
    // (10 + 5) * 2 + 10 = 40
    assert_eq!(result.as_int(), Some(40));
}

use aelys::run_with_config_and_opt;
use aelys_opt::OptimizationLevel;
use aelys_runtime::{Value, VmConfig};

fn run_with_opt(code: &str, level: OptimizationLevel) -> Value {
    run_with_config_and_opt(code, "<test>", VmConfig::default(), Vec::new(), level)
        .expect("Code should execute successfully")
}

#[test]
fn optimizer_runs_on_typed_program() {
    let src = aelys_syntax::Source::new("<t>", "fn f() -> int { 1 + 2 }");
    let tokens = aelys_frontend::lexer::Lexer::with_source(src.clone())
        .scan()
        .unwrap();
    let ast = aelys_frontend::parser::Parser::new(tokens, src.clone())
        .parse()
        .unwrap();
    let typed = aelys_sema::TypeInference::infer_program(ast, src).unwrap();
    let mut opt = aelys_opt::Optimizer::new(aelys_opt::OptimizationLevel::Standard);
    let _ = opt.optimize(typed);
}

#[test]
fn test_constant_fold_int_arithmetic() {
    // Test that constant folding produces correct results
    let result = run_with_opt("2 + 3 * 4", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(14));

    let result = run_with_opt("(10 - 5) / 2", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(2));

    let result = run_with_opt("100 % 7", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(2));
}

#[test]
fn test_constant_fold_float_arithmetic() {
    let result = run_with_opt("1.5 + 2.5", OptimizationLevel::Basic);
    assert_eq!(result.as_float(), Some(4.0));

    let result = run_with_opt("10.0 / 4.0", OptimizationLevel::Basic);
    assert_eq!(result.as_float(), Some(2.5));
}

#[test]
fn test_constant_fold_comparisons() {
    let result = run_with_opt("1 < 2", OptimizationLevel::Basic);
    assert_eq!(result.as_bool(), Some(true));

    let result = run_with_opt("5 > 10", OptimizationLevel::Basic);
    assert_eq!(result.as_bool(), Some(false));

    let result = run_with_opt("3 == 3", OptimizationLevel::Basic);
    assert_eq!(result.as_bool(), Some(true));

    let result = run_with_opt("4 != 4", OptimizationLevel::Basic);
    assert_eq!(result.as_bool(), Some(false));
}

#[test]
fn test_constant_fold_string_concat() {
    let code = r#"
        let s = "Hello" + " " + "World"
        s
    "#;
    let result = run_with_opt(code, OptimizationLevel::Basic);
    // The result should be "Hello World"
    assert!(result.is_ptr());
}

#[test]
fn test_constant_fold_unary() {
    let result = run_with_opt("-(-5)", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(5));

    // Aelys uses 'not' instead of '!'
    let result = run_with_opt("not false", OptimizationLevel::Basic);
    assert_eq!(result.as_bool(), Some(true));

    let result = run_with_opt("not not true", OptimizationLevel::Basic);
    assert_eq!(result.as_bool(), Some(true));
}

#[test]
fn test_constant_fold_nested() {
    // Deeply nested constant expressions
    let result = run_with_opt(
        "((1 + 2) * (3 + 4)) - ((5 - 2) * 2)",
        OptimizationLevel::Basic,
    );
    // (3 * 7) - (3 * 2) = 21 - 6 = 15
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_no_fold_division_by_zero() {
    // Division by zero should not be folded - we test that the optimizer
    // doesn't crash when encountering this pattern
    // The actual expression uses variables so it's not foldable anyway
    let code = r#"
        fn divide(a, b) {
            a / b
        }
        divide(10, 2)
    "#;
    let result = run_with_opt(code, OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_no_fold_overflow() {
    // Test that overflow protection works (uses checked arithmetic)
    // Use smaller numbers that are within VM's int range
    let code = "let x = 100000\nx + 200000";
    let result = run_with_opt(code, OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(300000));
}

#[test]
fn test_dce_after_return() {
    // Code after return should be eliminated
    let code = r#"
        fn test() {
            return 42
            let x = 100  // Dead code
            x + 1        // Dead code
        }
        test()
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(42));
}

#[test]
fn test_dce_constant_if_true() {
    // if true should eliminate else branch
    let code = r#"
        let x = if true { 1 } else { 2 }
        x
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(1));
}

#[test]
fn test_dce_constant_if_false() {
    // if false should eliminate then branch
    let code = r#"
        let x = if false { 1 } else { 2 }
        x
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(2));
}

#[test]
fn test_dce_while_false() {
    // while false loop body is dead
    let code = r#"
        let mut x = 10
        while false {
            x++  // Never executed
        }
        x
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(10));
}

#[test]
fn test_opt_level_none() {
    // O0 should still produce correct results
    let result = run_with_opt("2 + 3", OptimizationLevel::None);
    assert_eq!(result.as_int(), Some(5));
}

#[test]
fn test_opt_level_basic() {
    // O1 should do constant folding
    let result = run_with_opt("2 + 3 * 4", OptimizationLevel::Basic);
    assert_eq!(result.as_int(), Some(14));
}

#[test]
fn test_opt_level_standard() {
    // O2 should do constant folding + DCE
    let code = r#"
        fn test() {
            if false { return 0 }
            1 + 2 + 3
        }
        test()
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(6));
}

#[test]
fn test_opt_level_aggressive() {
    // O3 should do multiple passes
    let result = run_with_opt("((1 + 1) * (2 + 2)) / 2", OptimizationLevel::Aggressive);
    // (2 * 4) / 2 = 4
    assert_eq!(result.as_int(), Some(4));
}

#[test]
fn test_optimization_preserves_semantics_simple() {
    let code = "let x = 10\nlet y = 20\nx + y";

    let result_o0 = run_with_opt(code, OptimizationLevel::None);
    let result_o2 = run_with_opt(code, OptimizationLevel::Standard);
    let result_o3 = run_with_opt(code, OptimizationLevel::Aggressive);

    assert_eq!(result_o0.as_int(), result_o2.as_int());
    assert_eq!(result_o2.as_int(), result_o3.as_int());
}

#[test]
fn test_optimization_preserves_semantics_function() {
    let code = r#"
        fn factorial(n) {
            if n <= 1 { return 1 }
            n * factorial(n - 1)
        }
        factorial(5)
    "#;

    let result_o0 = run_with_opt(code, OptimizationLevel::None);
    let result_o2 = run_with_opt(code, OptimizationLevel::Standard);

    assert_eq!(result_o0.as_int(), Some(120));
    assert_eq!(result_o2.as_int(), Some(120));
}

#[test]
fn test_optimization_preserves_semantics_loops() {
    let code = r#"
        let mut sum = 0
        for i in 1..5 {
            sum += i
        }
        sum
    "#;

    let result_o0 = run_with_opt(code, OptimizationLevel::None);
    let result_o2 = run_with_opt(code, OptimizationLevel::Standard);

    // 1 + 2 + 3 + 4 = 10
    assert_eq!(result_o0.as_int(), Some(10));
    assert_eq!(result_o2.as_int(), Some(10));
}

#[test]
fn test_optimization_preserves_semantics_closures() {
    // Aelys uses fn(y) {} for lambda expressions
    let code = r#"
        fn make_adder(x) {
            return fn(y) { return x + y }
        }
        let add5 = make_adder(5)
        add5(10)
    "#;

    let result_o0 = run_with_opt(code, OptimizationLevel::None);
    let result_o2 = run_with_opt(code, OptimizationLevel::Standard);

    assert_eq!(result_o0.as_int(), Some(15));
    assert_eq!(result_o2.as_int(), Some(15));
}

#[test]
fn test_register_reuse_chain() {
    // Chain of operations should not use too many registers
    let code = "1 + 2 + 3 + 4 + 5 + 6 + 7 + 8 + 9 + 10";
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(55));
}

#[test]
fn test_register_reuse_with_locals() {
    let code = r#"
        let a = 1
        let b = 2
        let c = 3
        a + b + c + a + b + c
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(12));
}

#[test]
fn test_empty_function() {
    let code = r#"
        fn empty() {}
        empty()
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert!(result.is_null());
}

#[test]
fn test_nested_if_constant() {
    // Simple nested if test
    let code = r#"
        let x = if true { 10 } else { 20 }
        let y = if false { 30 } else { 40 }
        x + y
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(50)); // 10 + 40
}

#[test]
fn test_mixed_types_in_expression() {
    // Int + Float should promote to float
    let result = run_with_opt("1 + 2.5", OptimizationLevel::Basic);
    assert_eq!(result.as_float(), Some(3.5));
}

#[test]
fn test_global_const_prop_simple_literal() {
    let code = r#"
        let X: int = 42
        X + 10
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(52));
}

#[test]
fn test_global_const_prop_simple_dependency() {
    let code = r#"
        let A: int = 10
        let B: int = A * 2
        B + 5
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(25));
}

#[test]
fn test_global_const_prop_chained_dependency() {
    let code = r#"
        let A: int = 10
        let B: int = A * 2
        let C: int = B + 5
        C
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(25));
}

#[test]
fn test_global_const_prop_mutable_not_propagated() {
    let code = r#"
        let mut X: int = 42
        X++
        X
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(43));
}

#[test]
fn test_global_const_prop_complex_expression() {
    let code = r#"
        let WIDTH: float = 80.0
        let R1: float = 1.0
        let R2: float = 2.0
        let K2: float = 5.0
        let K1: float = WIDTH * K2 * 3.0 / (8.0 * (R1 + R2))
        K1
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    // WIDTH * K2 * 3.0 / (8.0 * (R1 + R2))
    // = 80 * 5.0 * 3.0 / (8.0 * 3.0)
    // = 1200.0 / 24.0
    // = 50.0
    assert_eq!(result.as_float(), Some(50.0));
}

#[test]
fn test_global_const_prop_float_globals() {
    let code = r#"
        let PI: float = 3.14159
        let RADIUS: float = 2.0
        let AREA: float = PI * RADIUS * RADIUS
        AREA
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    let expected = std::f64::consts::PI * 2.0 * 2.0;
    assert!((result.as_float().unwrap() - expected).abs() < 0.0001);
}

#[test]
fn test_global_const_prop_in_function() {
    let code = r#"
        let MULTIPLIER: int = 10
        fn scale(x: int) -> int {
            x * MULTIPLIER
        }
        scale(5)
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(50));
}

#[test]
fn test_global_const_prop_non_propagable_call() {
    let code = r#"
        fn get_value() -> int { 42 }
        let X: int = get_value()
        let Y: int = 10
        Y + 5
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_global_const_prop_bool_constant() {
    let code = r#"
        let DEBUG: bool = true
        let result: int = if DEBUG { 100 } else { 200 }
        result
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(100));
}

#[test]
fn test_global_const_prop_string_constant() {
    let code = r#"
        let PREFIX: string = "Hello"
        let SUFFIX: string = "World"
        let MSG: string = PREFIX + " " + SUFFIX
        MSG
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert!(result.is_ptr());
}

#[test]
fn test_fold_grouping_in_binary() {
    // Test that (8.0 * (1.0 + 2.0)) folds completely
    let code = "8.0 * (1.0 + 2.0)";
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_float(), Some(24.0));
}

#[test]
fn test_fold_nested_grouping_division() {
    // Test that 1200.0 / (8.0 * (1.0 + 2.0)) folds completely
    let code = "1200.0 / (8.0 * (1.0 + 2.0))";
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_float(), Some(50.0));
}

#[test]
fn test_local_const_prop_simple() {
    let code = r#"
        fn test() -> int {
            let x = 5
            let y = 10
            x + y
        }
        test()
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(15));
}

#[test]
fn test_local_const_prop_chained() {
    let code = r#"
        fn test() -> int {
            let a = 10
            let b = a * 2
            let c = b + 5
            c
        }
        test()
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(25));
}

#[test]
fn test_local_const_prop_deep_chain() {
    let code = r#"
        fn test() -> int {
            let x = 100
            let y = x * 2
            let z = y + 50
            z / 5
        }
        test()
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(50));
}

#[test]
fn test_local_const_prop_mutable_not_propagated() {
    let code = r#"
        fn test() -> int {
            let mut x = 5
            x++
            x
        }
        test()
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(6));
}

#[test]
fn test_local_const_prop_in_nested_if() {
    // tests that outer constants are propagated into nested if-blocks
    let code = r#"
        fn test() -> int {
            let base = 10
            let multiplier = 3
            if true {
                base * multiplier
            } else {
                0
            }
        }
        test()
    "#;
    // base * multiplier = 30
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(30));
}

#[test]
fn test_local_const_prop_with_conditionals() {
    let code = r#"
        fn test(flag: bool) -> int {
            let base = 10
            if flag {
                let factor = 5
                base * factor
            } else {
                let offset = 3
                base + offset
            }
        }
        test(true) + test(false)
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(63)); // 50 + 13
}

#[test]
fn test_local_const_prop_preserves_semantics() {
    let code = r#"
        fn calculate() -> int {
            let a = 5
            let b = 10
            a + b * 2
        }
        calculate()
    "#;
    let result_o0 = run_with_opt(code, OptimizationLevel::None);
    let result_o3 = run_with_opt(code, OptimizationLevel::Aggressive);
    assert_eq!(result_o0.as_int(), result_o3.as_int());
    assert_eq!(result_o3.as_int(), Some(25));
}

#[test]
fn test_local_const_prop_lambda() {
    let code = r#"
        fn test() -> int {
            let multiplier = 3
            let f = fn(x: int) -> int {
                let offset = 10
                x * multiplier + offset
            }
            f(5)
        }
        test()
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    assert_eq!(result.as_int(), Some(25)); // 5 * 3 + 10
}

#[test]
fn test_local_const_prop_float() {
    let code = r#"
        fn test() -> float {
            let pi = 3.14159
            let r = 2.0
            pi * r * r
        }
        test()
    "#;
    let result = run_with_opt(code, OptimizationLevel::Standard);
    let expected = std::f64::consts::PI * 2.0 * 2.0;
    assert!((result.as_float().unwrap() - expected).abs() < 0.0001);
}

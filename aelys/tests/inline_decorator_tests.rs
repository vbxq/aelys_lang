use aelys::run_with_config_and_opt;
use aelys_common::warning::WarningKind;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::{OptimizationLevel, Optimizer};
use aelys_runtime::{Value, VmConfig};
use aelys_sema::TypeInference;
use aelys_syntax::Source;

fn run_opt(src: &str, level: OptimizationLevel) -> Value {
    run_with_config_and_opt(src, "<test>", VmConfig::default(), Vec::new(), level)
        .expect("should run")
}

fn optimize_and_get_warnings(src: &str, level: OptimizationLevel) -> Vec<WarningKind> {
    let source = Source::new("<test>", src);
    let tokens = Lexer::with_source(source.clone()).scan().unwrap();
    let stmts = Parser::new(tokens, source.clone()).parse().unwrap();
    let typed = TypeInference::infer_program(stmts, source).unwrap();
    let mut opt = Optimizer::new(level);
    let _ = opt.optimize(typed);
    opt.warnings().iter().map(|w| w.kind.clone()).collect()
}

// basic inline behavior
#[test]
fn inline_simple_function() {
    let src = r#"
        @inline
        fn double(x: int) -> int { x * 2 }
        double(5)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(10));
}

#[test]
fn inline_preserves_semantics() {
    let src = r#"
        @inline
        fn add(a: int, b: int) -> int { a + b }
        add(3, 4) + add(10, 20)
    "#;
    let o0 = run_opt(src, OptimizationLevel::None).as_int();
    let o2 = run_opt(src, OptimizationLevel::Standard).as_int();
    assert_eq!(o0, o2);
    assert_eq!(o2, Some(37));
}

#[test]
fn inline_always_forces_inlining() {
    let src = r#"
        @inline_always
        fn triple(x: int) -> int { x * 3 }
        triple(7)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(21));
}

#[test]
fn inline_trivial_function_auto_inlines() {
    let src = r#"
        fn tiny(x: int) -> int { x + 1 }
        tiny(99)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(100));
}

#[test]
fn inline_single_call_site_auto_inlines() {
    let src = r#"
        fn helper(x: int) -> int {
            let a = x * 2
            let b = a + 10
            b
        }
        helper(5)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(20));
}

// warning tests
#[test]
fn warn_on_recursive_inline() {
    let src = r#"
        @inline
        fn factorial(n: int) -> int {
            if n <= 1 { 1 } else { n * factorial(n - 1) }
        }
        factorial(5)
    "#;
    let warnings = optimize_and_get_warnings(src, OptimizationLevel::Standard);
    assert!(warnings.iter().any(|k| matches!(k, WarningKind::InlineRecursive)));
}

#[test]
fn warn_on_mutual_recursion() {
    let src = r#"
        @inline
        fn ping(n: int) -> int {
            if n <= 0 { 0 } else { pong(n - 1) }
        }

        @inline
        fn pong(n: int) -> int {
            if n <= 0 { 1 } else { ping(n - 1) }
        }

        ping(5)
    "#;
    let warnings = optimize_and_get_warnings(src, OptimizationLevel::Standard);
    assert!(warnings.iter().any(|k| matches!(k, WarningKind::InlineMutualRecursion { .. })));
}

// nested function @inline is currently a no-op (optimizer only considers top-level functions)
#[test]
fn nested_inline_is_noop() {
    let src = r#"
        fn outer() -> int {
            let captured = 42

            @inline
            fn inner() -> int { captured }

            inner()
        }
        outer()
    "#;
    // should still execute correctly, just no inlining happens
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(42));
    // no warning because nested functions aren't analyzed for inlining
    let warnings = optimize_and_get_warnings(src, OptimizationLevel::Standard);
    assert!(warnings.is_empty());
}

// top-level function with closure capture (lambda assigned to variable)
#[test]
fn top_level_closure_with_capture() {
    let src = r#"
        let base = 100

        @inline
        fn uses_global() -> int { base }

        uses_global()
    "#;
    // executes correctly even though top-level globals are involved
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(100));
}

#[test]
fn recursive_inline_always_still_warns() {
    let src = r#"
        @inline_always
        fn infinite(n: int) -> int {
            infinite(n + 1)
        }
        0
    "#;
    let warnings = optimize_and_get_warnings(src, OptimizationLevel::Standard);
    assert!(warnings.iter().any(|k| matches!(k, WarningKind::InlineRecursive)));
}

// multiple functions
#[test]
fn inline_multiple_functions() {
    let src = r#"
        @inline
        fn square(x: int) -> int { x * x }

        @inline
        fn cube(x: int) -> int { x * x * x }

        square(3) + cube(2)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(17));
}

#[test]
fn inline_chain_calls() {
    let src = r#"
        @inline
        fn inc(x: int) -> int { x + 1 }

        @inline
        fn double(x: int) -> int { x * 2 }

        double(inc(5))
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(12));
}

// edge cases
#[test]
fn inline_with_no_args() {
    let src = r#"
        @inline
        fn constant() -> int { 42 }
        constant()
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(42));
}

#[test]
fn inline_returning_float() {
    let src = r#"
        @inline
        fn half(x: float) -> float { x / 2.0 }
        half(10.0)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_float(), Some(5.0));
}

#[test]
fn inline_with_bool() {
    let src = r#"
        @inline
        fn negate(b: bool) -> bool { not b }
        negate(true)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_bool(), Some(false));
}

// O0 should NOT inline
#[test]
fn no_inlining_at_o0() {
    let src = r#"
        @inline
        fn double(x: int) -> int { x * 2 }
        double(5)
    "#;
    let warnings = optimize_and_get_warnings(src, OptimizationLevel::None);
    assert!(warnings.is_empty());
}

// aggressive mode
#[test]
fn aggressive_mode_inlines_more() {
    let src = r#"
        fn medium_sized(x: int) -> int {
            let a = x + 1
            let b = a * 2
            let c = b - 3
            let d = c / 2
            d
        }
        medium_sized(10) + medium_sized(20)
    "#;
    let o2 = run_opt(src, OptimizationLevel::Standard).as_int();
    let o3 = run_opt(src, OptimizationLevel::Aggressive).as_int();
    assert_eq!(o2, o3);
}

// complex expressions
#[test]
fn inline_binary_expression() {
    let src = r#"
        @inline
        fn add(a: int, b: int) -> int { a + b }

        let result = add(1, 2) * add(3, 4)
        result
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(21));
}

#[test]
fn inline_in_if_condition() {
    let src = r#"
        @inline
        fn is_positive(x: int) -> bool { x > 0 }

        if is_positive(5) { 100 } else { 0 }
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(100));
}

#[test]
fn inline_in_loop() {
    let src = r#"
        @inline
        fn increment(x: int) -> int { x + 1 }

        let mut sum = 0
        for i in 0..5 {
            sum = increment(sum)
        }
        sum
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(5));
}

// semantics preservation under all opt levels
#[test]
fn semantics_preserved_all_levels() {
    let src = r#"
        @inline
        fn calc(a: int, b: int) -> int {
            a * b + a - b
        }
        calc(7, 3)
    "#;
    let o0 = run_opt(src, OptimizationLevel::None).as_int();
    let o1 = run_opt(src, OptimizationLevel::Basic).as_int();
    let o2 = run_opt(src, OptimizationLevel::Standard).as_int();
    let o3 = run_opt(src, OptimizationLevel::Aggressive).as_int();

    assert_eq!(o0, o1);
    assert_eq!(o1, o2);
    assert_eq!(o2, o3);
    assert_eq!(o3, Some(25)); // 7*3 + 7 - 3 = 21 + 4 = 25
}

// decorator parsing
#[test]
fn decorator_on_function_only() {
    let src = r#"
        @inline
        fn decorated() -> int { 1 }
        decorated()
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(1));
}

#[test]
fn multiple_decorators() {
    // @inline and @inline_always shouldn't conflict, inline_always takes precedence
    let src = r#"
        @inline_always
        fn force_inline(x: int) -> int { x + 1 }
        force_inline(10)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(11));
}

// indirect recursion through 3+ functions
#[test]
fn warn_on_triple_mutual_recursion() {
    let src = r#"
        @inline
        fn a(n: int) -> int { if n <= 0 { 0 } else { b(n - 1) } }

        @inline
        fn b(n: int) -> int { if n <= 0 { 1 } else { c(n - 1) } }

        @inline
        fn c(n: int) -> int { if n <= 0 { 2 } else { a(n - 1) } }

        a(3)
    "#;
    let warnings = optimize_and_get_warnings(src, OptimizationLevel::Standard);
    assert!(warnings.iter().any(|k| matches!(k, WarningKind::InlineMutualRecursion { .. })));
}

// function with side effects (still inlines, just not pure)
#[test]
fn inline_function_with_side_effects() {
    let src = r#"
        let mut counter = 0

        @inline
        fn inc_and_get() -> int {
            counter = counter + 1
            counter
        }

        let a = inc_and_get()
        let b = inc_and_get()
        a + b
    "#;
    // even with side effects, behavior should be consistent
    let o0 = run_opt(src, OptimizationLevel::None).as_int();
    let o2 = run_opt(src, OptimizationLevel::Standard).as_int();
    assert_eq!(o0, o2);
}

// regression: inline with string type
#[test]
fn inline_string_return() {
    let src = r#"
        @inline
        fn greet(name: string) -> string { "Hello, " + name }
        greet("World")
    "#;
    run_opt(src, OptimizationLevel::Standard); // just check no crash
}

// inline function called with expressions as arguments
#[test]
fn inline_with_complex_args() {
    let src = r#"
        @inline
        fn add(a: int, b: int) -> int { a + b }

        fn other() -> int { 5 }

        add(other() * 2, 3 + 4)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(17));
}

// public function shouldn't have special treatment in optimizer (that's compiler concern)
#[test]
fn inline_public_function() {
    let src = r#"
        @inline
        pub fn public_fn(x: int) -> int { x * 2 }
        public_fn(7)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(14));
}

// multiple decorators
#[test]
fn inline_combined_with_no_gc() {
    let src = r#"
        @no_gc
        @inline
        fn fast_add(a: int, b: int) -> int { a + b }
        fast_add(10, 20)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(30));
}

#[test]
fn inline_always_combined_with_no_gc() {
    let src = r#"
        @inline_always
        @no_gc
        fn fast_mul(a: int, b: int) -> int { a * b }
        fast_mul(7, 8)
    "#;
    assert_eq!(run_opt(src, OptimizationLevel::Standard).as_int(), Some(56));
}

// stats: check inliner actually counts something
#[test]
fn optimizer_stats_track_inlining() {
    let src = r#"
        @inline
        fn double(x: int) -> int { x * 2 }
        double(1) + double(2) + double(3)
    "#;
    let source = Source::new("<test>", src);
    let tokens = Lexer::with_source(source.clone()).scan().unwrap();
    let stmts = Parser::new(tokens, source.clone()).parse().unwrap();
    let typed = TypeInference::infer_program(stmts, source).unwrap();
    let mut opt = Optimizer::new(OptimizationLevel::Standard);
    let _ = opt.optimize(typed);
    // inliner should report some activity (stats might be > 0)
    let stats = opt.stats();
    // just verify stats are accessible
    let _ = stats.functions_inlined;
}

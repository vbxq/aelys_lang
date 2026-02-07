mod common;

use aelys_bytecode::asm::disassemble;
use aelys_driver::pipeline::{compilation_pipeline, compilation_pipeline_with_opt};
use aelys_opt::OptimizationLevel;
use aelys_runtime::{VM, stdlib};
use aelys_syntax::Source;
use common::assert_aelys_int;

#[test]
fn test_builtin_call_in_loop_uses_cache() {
    assert_aelys_int(
        r#"
        let ptr = alloc(100)
        for i in 0..100 {
            store(ptr, i, i * 2)
        }
        load(ptr, 50)
    "#,
        100,
    );
}

#[test]
fn test_recursive_function_with_cache() {
    assert_aelys_int(
        r#"
        fn fib(n: int) -> int {
            if n <= 1 { return n }
            return fib(n - 1) + fib(n - 2)
        }
        fib(20)
    "#,
        6765,
    );
}

#[test]
fn test_call_global_mono_patches_correctly() {
    assert_aelys_int(
        r#"
        fn increment(x: int) -> int { return x + 1 }
        let mut sum = 0
        for i in 0..1000 {
            sum = increment(sum)
        }
        sum
    "#,
        1000,
    );
}

#[test]
fn test_call_global_native_patches_correctly() {
    assert_aelys_int(
        r#"
        let ptr = alloc(1)
        let mut val = 0
        for i in 0..1000 {
            store(ptr, 0, i)
            val = load(ptr, 0)
        }
        val
    "#,
        999,
    );
}

#[test]
fn test_nested_function_calls_with_cache() {
    assert_aelys_int(
        r#"
        fn add(a: int, b: int) -> int { return a + b }
        fn mul(a: int, b: int) -> int { return a * b }
        fn compute(x: int) -> int { return add(mul(x, 2), mul(x, 3)) }
        compute(10)
    "#,
        50,
    );
}

#[test]
fn test_alternating_function_calls() {
    assert_aelys_int(
        r#"
        fn even_step(x: int) -> int { return x + 2 }
        fn odd_step(x: int) -> int { return x + 1 }
        let mut val = 0
        for i in 0..100 {
            if i % 2 == 0 {
                val = even_step(val)
            } else {
                val = odd_step(val)
            }
        }
        val
    "#,
        150,
    );
}

#[test]
fn test_cache_with_different_arities() {
    assert_aelys_int(
        r#"
        fn zero() -> int { return 0 }
        fn one(a: int) -> int { return a }
        fn two(a: int, b: int) -> int { return a + b }
        fn three(a: int, b: int, c: int) -> int { return a + b + c }

        let mut sum = 0
        for i in 0..100 {
            sum = sum + zero() + one(1) + two(1, 2) + three(1, 2, 3)
        }
        sum
    "#,
        1000,
    );
}

#[test]
fn test_disassembler_skips_cache_words() {
    // use O0 to prevent inlining so we can test call opcodes
    let mut pipeline = compilation_pipeline_with_opt(OptimizationLevel::None);

    let source = r#"
        fn foo() -> int { return 42 }
        foo() + foo()
    "#;

    let src = Source::new("test", source);
    let (func, _heap) = pipeline.compile(src).expect("compile failed");
    let output = disassemble(&func, None);

    let lines: Vec<&str> = output.lines().collect();
    let mut prev_offset: Option<usize> = None;
    let mut found_gap = false;

    for line in &lines {
        let trimmed = line.trim();
        if trimmed.starts_with(|c: char| c.is_ascii_digit())
            && let Some(offset_str) = trimmed.split(':').next()
            && let Ok(offset) = offset_str.parse::<usize>()
        {
            if let Some(prev) = prev_offset
                && offset > prev
                && offset - prev == 3
            {
                found_gap = true;
            }
            prev_offset = Some(offset);
        }
    }

    assert!(
        found_gap,
        "Disassembler should show gaps of 3 (instruction + 2 cache words). Output:\n{}",
        output
    );
}

#[test]
fn test_module_exports_contains_native_functions() {
    let source = Source::new("<test>", "");
    let mut vm = VM::new(source).unwrap();

    let exports = stdlib::register_std_module(&mut vm, "math").expect("register math failed");

    assert!(
        !exports.native_functions.is_empty(),
        "math module should export native functions"
    );

    let has_sqrt = exports.native_functions.iter().any(|n| n.contains("sqrt"));
    let has_sin = exports.native_functions.iter().any(|n| n.contains("sin"));
    let has_cos = exports.native_functions.iter().any(|n| n.contains("cos"));

    assert!(has_sqrt, "math should export sqrt");
    assert!(has_sin, "math should export sin");
    assert!(has_cos, "math should export cos");
}

#[test]
fn test_stdlib_modules_export_native_functions() {
    let modules = ["math", "io", "string", "convert", "time"];
    let source = Source::new("<test>", "");

    for module_name in &modules {
        let mut vm = VM::new(source.clone()).unwrap();
        let exports = stdlib::register_std_module(&mut vm, module_name)
            .unwrap_or_else(|_| panic!("register {} failed", module_name));

        assert!(
            !exports.native_functions.is_empty(),
            "{} module should export native functions, got empty list",
            module_name
        );

        for func_name in &exports.native_functions {
            assert!(
                func_name.starts_with(&format!("{}::", module_name)),
                "Function '{}' should be prefixed with '{}::'",
                func_name,
                module_name
            );
        }
    }
}

#[test]
fn test_builtin_functions_work() {
    let source = r#"
        let p = alloc(8)
        store(p, 0, 100)
        let v = load(p, 0)
        free(p)
        v
    "#;

    assert_aelys_int(source, 100);
}

#[test]
fn test_heavy_builtin_call_loop() {
    assert_aelys_int(
        r#"
        let ptr = alloc(1)
        store(ptr, 0, 0)
        for i in 0..10000 {
            let v = load(ptr, 0)
            store(ptr, 0, v + 1)
        }
        load(ptr, 0)
    "#,
        10000,
    );
}

#[test]
fn test_interleaved_aelys_and_builtin_calls() {
    assert_aelys_int(
        r#"
        fn increment(x: int) -> int { return x + 1 }
        fn decrement(x: int) -> int { return x - 1 }

        let ptr = alloc(1)
        store(ptr, 0, 0)

        for i in 0..1000 {
            let v = load(ptr, 0)
            let v2 = increment(v)
            let v3 = increment(v2)
            let v4 = decrement(v3)
            store(ptr, 0, v4)
        }

        load(ptr, 0)
    "#,
        1000,
    );
}

#[test]
fn test_memory_opcodes_in_bytecode() {
    let mut pipeline = compilation_pipeline();

    let source = r#"
        let ptr = alloc(10)
        store(ptr, 0, 42)
        load(ptr, 0)
    "#;

    let src = Source::new("test", source);
    let (func, _heap) = pipeline.compile(src).expect("compile failed");

    let mut found_alloc = false;
    let mut found_store = false;
    let mut found_load = false;

    for &instr in func.bytecode.as_slice() {
        let opcode = (instr >> 24) as u8;
        match opcode {
            28 => found_alloc = true,      // Alloc
            32 | 33 => found_store = true, // StoreMem / StoreMemI
            30 | 31 => found_load = true,  // LoadMem / LoadMemI
            _ => {}
        }
    }

    assert!(found_alloc, "Expected Alloc opcode (28) in bytecode");
    assert!(
        found_store,
        "Expected StoreMem/StoreMemI opcode (32/33) in bytecode"
    );
    assert!(
        found_load,
        "Expected LoadMem/LoadMemI opcode (30/31) in bytecode"
    );
}

#[test]
fn test_call_global_opcode_for_aelys_functions() {
    // use O0 to prevent inlining so we can verify CallGlobal opcodes
    let mut pipeline = compilation_pipeline_with_opt(OptimizationLevel::None);

    let source = r#"
        fn double(x: int) -> int { return x * 2 }
        fn triple(x: int) -> int { return x * 3 }
        double(5) + triple(3)
    "#;

    let src = Source::new("test", source);
    let (func, _heap) = pipeline.compile(src).expect("compile failed");

    let mut found_call_global = 0;
    for &instr in func.bytecode.as_slice() {
        let opcode = (instr >> 24) as u8;
        if opcode == 77 {
            found_call_global += 1;
        }
    }

    assert!(
        found_call_global >= 2,
        "Expected at least 2 CallGlobal opcodes (double/triple), found {}",
        found_call_global
    );
}

#[test]
fn test_cache_words_present_after_call_opcodes() {
    // use O0 to prevent inlining
    let mut pipeline = compilation_pipeline_with_opt(OptimizationLevel::None);

    let source = r#"
        fn foo() -> int { return 42 }
        foo()
    "#;

    let src = Source::new("test", source);
    let (func, _heap) = pipeline.compile(src).expect("compile failed");

    let bytecode = func.bytecode.as_slice();
    let mut call_positions = Vec::new();
    for (i, &instr) in bytecode.iter().enumerate() {
        let opcode = (instr >> 24) as u8;
        if opcode == 77 || opcode == 78 || opcode == 104 {
            call_positions.push(i);
        }
    }

    for pos in call_positions {
        assert!(
            pos + 2 < bytecode.len(),
            "Call opcode at position {} should have 2 cache words following it",
            pos
        );
    }
}

#[test]
fn test_aelys_function_repeated_calls_same_result() {
    assert_aelys_int(
        r#"
        fn double(x: int) -> int { return x * 2 }
        let a = double(5)
        let b = double(5)
        let c = double(5)
        a + b + c
    "#,
        30,
    );
}

#[test]
fn test_type_builtin_uses_cache() {
    assert_aelys_int(
        r#"
        let t1 = type(42)
        let t2 = type(3.14)
        let t3 = type("hello")
        let t4 = type(true)
        42
    "#,
        42,
    );
}

#[test]
fn test_mixed_aelys_and_builtin_in_expression() {
    assert_aelys_int(
        r#"
        fn square(x: int) -> int { return x * x }
        let ptr = alloc(1)
        store(ptr, 0, square(5))
        load(ptr, 0) + square(3)
    "#,
        34,
    );
}

#[test]
fn test_call_in_conditional() {
    assert_aelys_int(
        r#"
        fn is_even(n: int) -> bool { return n % 2 == 0 }
        fn double(n: int) -> int { return n * 2 }
        fn triple(n: int) -> int { return n * 3 }

        let mut sum = 0
        for i in 0..10 {
            if is_even(i) {
                sum = sum + double(i)
            } else {
                sum = sum + triple(i)
            }
        }
        sum
    "#,
        115,
    );
}

#[test]
fn test_deeply_nested_calls() {
    assert_aelys_int(
        r#"
        fn a(x: int) -> int { return x + 1 }
        fn b(x: int) -> int { return a(x) + 1 }
        fn c(x: int) -> int { return b(x) + 1 }
        fn d(x: int) -> int { return c(x) + 1 }
        fn e(x: int) -> int { return d(x) + 1 }
        e(0)
    "#,
        5,
    );
}

#[test]
fn test_function_call_chain() {
    assert_aelys_int(
        r#"
        fn step1(n: int) -> int {
            if n <= 0 { return 0 }
            return step2(n - 1) + 1
        }
        fn step2(n: int) -> int {
            if n <= 0 { return 0 }
            return step1(n - 1) + 2
        }
        step1(5)
    "#,
        7,
    );
}

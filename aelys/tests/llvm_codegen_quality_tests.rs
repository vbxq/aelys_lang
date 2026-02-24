use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use std::fs;
use tempfile::tempdir;

fn compile_to_verified_ir(source: &str) -> String {
    compile_to_verified_ir_with_opt(source, OptimizationLevel::Standard)
}

fn compile_to_verified_ir_with_opt(source: &str, opt: OptimizationLevel) -> String {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, source).expect("source should be written");

    compile_file_with_llvm(&source_path, opt, true)
        .expect("llvm backend compilation should succeed");

    let ll_path = source_path.with_extension("ll");
    let ir = fs::read_to_string(&ll_path).expect("llvm ir file should be generated");

    let context = Context::create();
    let buffer = MemoryBuffer::create_from_file(&ll_path).expect("llvm ir should be readable");
    let module = context
        .create_module_from_ir(buffer)
        .expect("llvm ir should parse into a module");
    module
        .verify()
        .expect("module.verify() should succeed for generated ir");

    ir
}

fn assert_all_aligned(ir: &str, needle: &str, expected_align: u32) {
    let matching: Vec<_> = ir.lines().filter(|line| line.contains(needle)).collect();
    if matching.is_empty() {
        return;
    }

    let required = format!("align {expected_align}");
    for line in matching {
        assert!(
            line.contains(&required),
            "bad alignment for `{needle}`: {line}"
        );
    }
}

#[test]
fn llvm_uses_expected_alignment_for_mutable_locals() {
    let ir = compile_to_verified_ir_with_opt(
        r#"
fn int_align(n: i64) -> i64 {
    let a64: i64 = 0
    let a32: i32 = 0
    let a16: i16 = 0
    let a8: i8 = 0
    let ab: bool = false
    while a64 < n {
        a8 = a8 + (1 as i8)
        a16 = a16 + (1 as i16)
        a32 = a32 + 1
        a64 = a64 + 1
        if ab { ab = false } else { ab = true }
    }
    return a64 + (a32 as i64) + (a16 as i64) + (a8 as i64) + (ab as i64)
}

fn float_align(n: i64) -> f64 {
    let af32: f32 = 0.0 as f32
    let af64: f64 = 0.0
    let i: i64 = 0
    while i < n {
        af32 = af32 + (1.0 as f32)
        af64 = af64 + 1.0
        i = i + 1
    }
    return af64 + (af32 as f64)
}

fn string_align(s: string, n: i64) -> string {
    let sp: string = s
    let i: i64 = 0
    while i < n {
        sp = s
        i = i + 1
    }
    return sp
}
"#,
        OptimizationLevel::None,
    );

    assert_all_aligned(&ir, "alloca i8,", 1);
    assert_all_aligned(&ir, "alloca i16,", 2);
    assert_all_aligned(&ir, "alloca i32,", 4);
    assert_all_aligned(&ir, "alloca i64,", 8);
    assert_all_aligned(&ir, "alloca i1,", 1);
    assert_all_aligned(&ir, "alloca float,", 4);
    assert_all_aligned(&ir, "alloca double,", 8);
    assert_all_aligned(&ir, "alloca %__aelys_string,", 8);

    assert_all_aligned(&ir, "store i8 ", 1);
    assert_all_aligned(&ir, "store i16 ", 2);
    assert_all_aligned(&ir, "store i32 ", 4);
    assert_all_aligned(&ir, "store i64 ", 8);
    assert_all_aligned(&ir, "store i1 ", 1);
    assert_all_aligned(&ir, "store float ", 4);
    assert_all_aligned(&ir, "store double ", 8);
    assert_all_aligned(&ir, "store %__aelys_string ", 8);

    assert_all_aligned(&ir, "load i8,", 1);
    assert_all_aligned(&ir, "load i16,", 2);
    assert_all_aligned(&ir, "load i32,", 4);
    assert_all_aligned(&ir, "load i64,", 8);
    assert_all_aligned(&ir, "load i1,", 1);
    assert_all_aligned(&ir, "load float,", 4);
    assert_all_aligned(&ir, "load double,", 8);
    assert_all_aligned(&ir, "load %__aelys_string,", 8);

    let any_alloca = ir.lines().any(|l| l.contains("alloca"));
    assert!(
        any_alloca,
        "mutable locals in loops must produce alloca instructions:\n{ir}"
    );
}

#[test]
fn llvm_ssa_params_skip_alloca() {
    let ir = compile_to_verified_ir(
        r#"
fn add(a: i64, b: i64) -> i64 {
    return a + b
}

fn identity(x: i64) -> i64 {
    return x
}
"#,
    );

    assert!(
        !ir.contains("alloca"),
        "pure SSA functions must not generate alloca:\n{ir}"
    );
}

#[test]
fn llvm_verify_passes_for_fibonacci_sum_and_vec2_length() {
    let programs = [
        r#"
fn fibonacci(n: i32) -> i64 {
    if n <= 1 {
        return n as i64
    }
    return fibonacci(n - 1) + fibonacci(n - 2)
}
"#,
        r#"
fn sum(n: i32) -> i64 {
    let acc: i64 = 0
    let i: i32 = 0
    while i < n {
        acc = acc + i as i64
        i = i + 1
    }
    return acc
}
"#,
        r#"
struct Vec2 { x: f64, y: f64 }

fn vec2_length(v: Vec2) -> f64 {
    return v.x * v.x + v.y * v.y
}
"#,
    ];

    for program in programs {
        let ir = compile_to_verified_ir(program);
        assert!(ir.contains("define"));
    }
}

#[test]
fn llvm_calls_match_fastcc_declarations() {
    let ir = compile_to_verified_ir_with_opt(
        r#"
fn callee(x: i64) -> i64 {
    return x + 1
}

fn caller(v: i64) -> i64 {
    return callee(v)
}
"#,
        OptimizationLevel::None,
    );

    assert!(ir.contains("define fastcc i64 @callee"));
    assert!(ir.contains("call fastcc i64 @callee"));
}

//! not yet implemented yet:
//! - Stack array -> slice coercion

use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use std::fs;
use tempfile::tempdir;

fn compile_to_verified_ir(source: &str) -> String {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, source).expect("source should be written");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
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

#[test]
fn array_literal_inferred() {
    let ir = compile_to_verified_ir(
        r#"
fn second() -> i64 {
    let arr = [10, 20, 30]
    return arr[1]
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x i64]"),
        "should allocate [3 x i64] on the stack:\n{ir}"
    );
    assert!(
        ir.contains("store i64 10"),
        "should store first element 10:\n{ir}"
    );
    assert!(
        ir.contains("store i64 20"),
        "should store second element 20:\n{ir}"
    );
    assert!(
        ir.contains("store i64 30"),
        "should store third element 30:\n{ir}"
    );
    assert!(
        ir.contains("getelementptr"),
        "should generate GEP for index access:\n{ir}"
    );
    // No runtime array calls
    assert!(
        !ir.contains("__aelys_array_new"),
        "should NOT call __aelys_array_new (stack-allocated):\n{ir}"
    );
}

#[test]
fn array_literal_annotated() {
    let ir = compile_to_verified_ir(
        r#"
fn second() -> i64 {
    let arr: [i64; 3] = [10, 20, 30]
    return arr[1]
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x i64]"),
        "annotated array should allocate [3 x i64]:\n{ir}"
    );
    assert!(
        ir.contains("store i64 10"),
        "should store first element 10:\n{ir}"
    );
    assert!(
        ir.contains("store i64 20"),
        "should store second element 20:\n{ir}"
    );
    assert!(
        ir.contains("store i64 30"),
        "should store third element 30:\n{ir}"
    );
}

#[test]
fn array_annotation_f64() {
    let ir = compile_to_verified_ir(
        r#"
fn first() -> f64 {
    let arr: [f64; 2] = [1.0, 2.0]
    return arr[0]
}
"#,
    );
    assert!(
        ir.contains("alloca [2 x double]"),
        "f64 annotated array should allocate [2 x double]:\n{ir}"
    );
    assert!(
        ir.contains("store double"),
        "should store double values:\n{ir}"
    );
}

#[test]
fn array_repeat_syntax() {
    let ir = compile_to_verified_ir(
        r#"
fn first_zero() -> i64 {
    let zeros = [0; 100]
    return zeros[0]
}
"#,
    );
    assert!(
        ir.contains("alloca [100 x i64]"),
        "should allocate [100 x i64] on the stack:\n{ir}"
    );
    let store_count = ir.matches("store i64 0").count();
    assert!(
        store_count >= 100,
        "[0; 100] should generate at least 100 stores, got {store_count}:\n{ir}"
    );
}

#[test]
fn array_repeat_syntax_nonzero_fill() {
    let ir = compile_to_verified_ir(
        r#"
fn first_val() -> i64 {
    let arr = [42; 5]
    return arr[0]
}
"#,
    );
    assert!(
        ir.contains("alloca [5 x i64]"),
        "should allocate [5 x i64] on the stack:\n{ir}"
    );
    let store_count = ir.matches("store i64 42").count();
    assert!(
        store_count >= 5,
        "[42; 5] should store 42 into all 5 elements, got {store_count}:\n{ir}"
    );
}

#[test]
fn array_repeat_syntax_small() {
    let ir = compile_to_verified_ir(
        r#"
fn val() -> i64 {
    let arr = [7; 3]
    return arr[2]
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x i64]"),
        "should allocate [3 x i64]:\n{ir}"
    );
    let store_count = ir.matches("store i64 7").count();
    assert!(
        store_count >= 3,
        "[7; 3] should have 3 stores of 7, got {store_count}:\n{ir}"
    );
}

#[test]
fn array_f64() {
    let ir = compile_to_verified_ir(
        r#"
fn first_f() -> f64 {
    let arr = [1.5, 2.7, 3.14]
    return arr[0]
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x double]"),
        "f64 array should allocate [3 x double]:\n{ir}"
    );
    assert!(
        ir.contains("store double"),
        "should store double values:\n{ir}"
    );
    assert!(
        ir.contains("getelementptr"),
        "should use GEP for index:\n{ir}"
    );
}

#[test]
fn array_bool() {
    let ir = compile_to_verified_ir(
        r#"
fn first_b() -> bool {
    let arr = [true, false, true]
    return arr[0]
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x i1]"),
        "bool array should allocate [3 x i1]:\n{ir}"
    );
    assert!(ir.contains("store i1"), "should store i1 values:\n{ir}");
}

#[test]
fn array_param_syntax() {
    let ir = compile_to_verified_ir(
        r#"
fn process(data: [i64; 3]) -> i64 {
    return data[0]
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x i64]"),
        "array param should allocate [3 x i64]:\n{ir}"
    );
    assert!(
        ir.contains("getelementptr"),
        "should generate GEP for index access:\n{ir}"
    );
}

#[test]
#[should_panic(expected = "cannot return stack-allocated array")]
fn array_return_forbidden() {
    compile_to_verified_ir(
        r#"
fn bad() -> [i64; 3] {
    let arr = [1, 2, 3]
    return arr
}
"#,
    );
}

#[test]
#[should_panic(expected = "stack array too large")]
fn array_stack_overflow_repeat() {
    compile_to_verified_ir(
        r#"
fn huge() -> i64 {
    let big = [0; 200000]
    return big[0]
}
"#,
    );
}

#[test]
#[should_panic(expected = "stack array too large")]
fn array_stack_overflow_let_optimization() {
    // tests the stack size check in the Let-optimization path (stmts.rs)
    compile_to_verified_ir(
        r#"
fn huge_let() -> i64 {
    let big = [0; 500000]
    return big[0]
}
"#,
    );
}

#[test]
fn array_index_read() {
    let ir = compile_to_verified_ir(
        r#"
fn third() -> i64 {
    let arr = [100, 200, 300]
    return arr[2]
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x i64]"),
        "should use stack allocation:\n{ir}"
    );
    assert!(
        ir.contains("getelementptr"),
        "index read should use GEP:\n{ir}"
    );
    assert!(
        ir.contains("@__aelys_panic"),
        "should have bounds check panic:\n{ir}"
    );
}

#[test]
fn array_index_write() {
    let ir = compile_to_verified_ir(
        r#"
fn mutate() -> i64 {
    let arr = [10, 20, 30]
    arr[1] = 99
    return arr[1]
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x i64]"),
        "should use stack allocation:\n{ir}"
    );
    assert!(
        ir.contains("store i64 99"),
        "index write should store 99:\n{ir}"
    );
}

#[test]
fn array_foreach_known_length() {
    let ir = compile_to_verified_ir(
        r#"
fn sum_arr() -> i64 {
    let arr = [1, 2, 3]
    let total: i64 = 0
    for x in arr {
        total = total + x
    }
    return total
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x i64]"),
        "array should be stack-allocated:\n{ir}"
    );
    assert!(
        !ir.contains("__aelys_len"),
        "for-each on stack array should use known length, not __aelys_len:\n{ir}"
    );
}

#[test]
fn array_let_optimization_literal() {
    let ir = compile_to_verified_ir(
        r#"
fn get() -> i64 {
    let arr = [5, 10, 15]
    return arr[0]
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x i64]"),
        "should allocate [3 x i64]:\n{ir}"
    );
    assert!(ir.contains("store i64 5"), "should store 5:\n{ir}");
    assert!(ir.contains("store i64 10"), "should store 10:\n{ir}");
    assert!(ir.contains("store i64 15"), "should store 15:\n{ir}");
}

#[test]
fn array_let_optimization_repeat() {
    let ir = compile_to_verified_ir(
        r#"
fn get() -> i64 {
    let arr = [99; 10]
    return arr[5]
}
"#,
    );
    assert!(
        ir.contains("alloca [10 x i64]"),
        "should allocate [10 x i64]:\n{ir}"
    );
    let store_count = ir.matches("store i64 99").count();
    assert!(
        store_count >= 10,
        "[99; 10] should produce at least 10 stores, got {store_count}:\n{ir}"
    );
}

#[test]
fn array_single_element() {
    let ir = compile_to_verified_ir(
        r#"
fn only() -> i64 {
    let arr = [42]
    return arr[0]
}
"#,
    );
    assert!(
        ir.contains("alloca [1 x i64]"),
        "single-element array should allocate [1 x i64]:\n{ir}"
    );
    assert!(ir.contains("store i64 42"), "should store 42:\n{ir}");
}

#[test]
fn array_in_loop_body() {
    let ir = compile_to_verified_ir(
        r#"
fn use_in_loop() -> i64 {
    let result: i64 = 0
    let i: i64 = 0
    while i < 3 {
        let arr = [i, i, i]
        result = result + arr[0]
        i = i + 1
    }
    return result
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x i64]"),
        "array in loop should still be stack-allocated:\n{ir}"
    );
}

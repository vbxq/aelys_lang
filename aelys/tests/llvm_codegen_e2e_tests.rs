/* i know those are bad ways to check for the IR */

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
    compile_file_with_llvm(&source_path, OptimizationLevel::Standard, true)
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

fn all_i64_stores_align8(ir: &str) -> bool {
    ir.lines()
        .filter(|line| line.contains("store i64"))
        .all(|line| line.contains("align 8"))
}

fn has_back_edge(ir: &str) -> bool {
    let mut current_block = None;
    for line in ir.lines() {
        let trimmed = line.trim_start();
        if let Some(id) = parse_block_label(trimmed) {
            current_block = Some(id);
            continue;
        }
        let Some(from) = current_block else {
            continue;
        };
        for target in parse_branch_targets(trimmed) {
            if target <= from {
                return true;
            }
        }
    }
    false
}

fn parse_block_label(line: &str) -> Option<u32> {
    let rest = line.strip_prefix("bb")?;
    let (digits, _) = rest.split_once(':')?;
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn parse_branch_targets(line: &str) -> Vec<u32> {
    let mut targets = Vec::new();
    let mut remaining = line;
    while let Some(pos) = remaining.find("%bb") {
        let after = &remaining[(pos + 3)..];
        let digits: String = after.chars().take_while(|c| c.is_ascii_digit()).collect();
        if digits.is_empty() {
            if after.is_empty() {
                break;
            }
            remaining = &after[1..];
            continue;
        }
        if let Ok(id) = digits.parse() {
            targets.push(id);
        }
        remaining = &after[digits.len()..];
    }
    targets
}

#[test]
fn llvm_returns_integer_constant() {
    let ir = compile_to_verified_ir("fn constant() -> i64 { return 42 }");
    assert!(ir.contains("ret i64"));
}

#[test]
fn llvm_adds_two_parameters() {
    let ir = compile_to_verified_ir("fn add(a: i64, b: i64) -> i64 { return a + b }");
    assert!(ir.contains("add i64"));
    assert!(all_i64_stores_align8(&ir));
}

#[test]
fn llvm_generates_conditional_branch() {
    let ir = compile_to_verified_ir("fn choose(x: i64) -> i64 { if x > 0 { return 1 } return 2 }");
    assert!(ir.contains("br i1"));
}

#[test]
fn llvm_generates_while_back_edge() {
    let ir = compile_to_verified_ir(
        r#"
fn count(n: i64) -> i64 {
    let i: i64 = 0
    while i < n {
        i = i + 1
    }
    return i
}
"#,
    );
    assert!(has_back_edge(&ir));
}

#[test]
fn llvm_generates_sitofp_for_i32_to_f64() {
    let ir = compile_to_verified_ir(
        r#"
fn cast_it(x: i32) -> f64 {
    return x as f64
}
"#,
    );
    assert!(ir.contains("sitofp"), "{ir}");
}

#[test]
fn llvm_generates_gep_for_struct_init_and_access() {
    let ir = compile_to_verified_ir(
        r#"
struct Point { x: i64, y: i64 }
fn read_x() -> i64 {
    let p = Point { x: 1, y: 2 }
    return p.x
}
"#,
    );
    assert!(ir.contains("getelementptr"));
}

#[test]
fn llvm_generates_function_call() {
    let ir = compile_to_verified_ir(
        "fn callee(x: i64) -> i64 { return x } fn caller() -> i64 { return callee(7) }",
    );
    assert!(ir.contains("call"));
    assert!(ir.contains("@callee"));
}

#[test]
fn llvm_emits_global_string_constant() {
    let ir = compile_to_verified_ir("fn hello() -> string { return \"hello\" }");
    assert!(ir.contains("@str_"));
    assert!(ir.contains("hello"));
}

#[test]
fn llvm_void_function_without_explicit_return_uses_ret_void() {
    let ir = compile_to_verified_ir(
        r#"
fn unit_like() -> void {
    let x: i64 = 1
}
"#,
    );
    assert!(ir.contains("ret void"), "{ir}");
    assert!(!ir.contains("ret i64 0"));
    assert!(all_i64_stores_align8(&ir));
}

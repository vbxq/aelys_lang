use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use std::fs;
use tempfile::tempdir;

fn compile_source_to_verified_ir_with_opt(source: &str, opt: OptimizationLevel) -> String {
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

fn compile_source_to_verified_ir(source: &str) -> String {
    compile_source_to_verified_ir_with_opt(source, OptimizationLevel::Standard)
}

fn compile_source_expect_error(source: &str) -> String {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, source).expect("source should be written");

    match compile_file_with_llvm(&source_path, OptimizationLevel::Standard, true) {
        Ok(()) => panic!("expected llvm compilation to fail"),
        Err(err) => err.to_string(),
    }
}

#[test]
fn llvm_rejects_user_defined_reserved_bootstrap_println() {
    let error = compile_source_expect_error(
        r#"
fn println(s: string) {
    return
}
"#,
    );
    assert!(
        error.contains("reserved builtin during bootstrap") && error.contains("println"),
        "{error}"
    );
}

#[test]
fn llvm_println_bootstrap_uses_write_ptr_len_only() {
    let ir = compile_source_to_verified_ir(
        r#"
fn greet() {
    println("Hello")
}
"#,
    );

    assert!(ir.contains("declare void @__aelys_write(ptr, i64)"), "{ir}");
    assert!(!ir.contains("declare i64 @println("), "{ir}");
    assert!(!ir.contains("declare void @println("), "{ir}");
    assert!(!ir.contains("declare i64 @print("), "{ir}");
    assert!(!ir.contains("declare void @print("), "{ir}");
    assert!(!ir.contains("@println("), "{ir}");
    assert!(!ir.contains("@print("), "{ir}");
    assert!(ir.contains("c\"Hello\\00\""), "{ir}");
    assert!(!ir.contains("c\"\\00Hello"), "{ir}");
    assert!(!ir.contains(", i64 0, i64 1"), "{ir}");
    assert!(ir.contains("call void @__aelys_write"), "{ir}");
    assert!(ir.contains("i64 5"), "{ir}");
}

#[test]
fn llvm_user_str_param_is_passed_as_aelys_string_struct() {
    let ir = compile_source_to_verified_ir_with_opt(
        r#"
fn sink(s: string) -> i64 {
    print(s)
    return 0
}

fn caller() -> i64 {
    return sink("Hello")
}
"#,
        OptimizationLevel::None,
    );

    assert!(
        ir.contains("define fastcc i64 @sink(%__aelys_string"),
        "{ir}"
    );
    assert!(!ir.contains("define fastcc i64 @sink(ptr"), "{ir}");
    assert!(ir.contains("call fastcc i64 @sink(%__aelys_string"), "{ir}");
}

#[test]
fn unknown_field_on_str() {
    let error = compile_source_expect_error(
        r#"
fn main() -> i64 {
    let s = "Hello"
    return s.foo
}
"#,
    );
    assert!(
        error.contains("unknown field 'foo' on Str; supported: 'len'"),
        "{error}"
    );
}

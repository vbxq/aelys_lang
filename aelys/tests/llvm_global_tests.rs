use aelys_air::layout::compute_layouts;
use aelys_air::lower::lower;
use aelys_air::mono::monomorphize;
use aelys_air::passes::copy_elim::eliminate_copies;
use aelys_air::passes::dead_locals::eliminate_dead_locals;
use aelys_air::passes::validate::validate_air;
use aelys_codegen::CodegenContext;
use aelys_driver::compile_file_with_llvm;
use aelys_frontend::lexer::Lexer;
use aelys_frontend::parser::Parser;
use aelys_opt::OptimizationLevel;
use aelys_sema::TypeInference;
use aelys_syntax::Source;
use inkwell::context::Context;
use inkwell::memory_buffer::MemoryBuffer;
use std::fs;
use tempfile::tempdir;

fn compile_source_to_verified_ir_without_link(source: &str) -> String {
    let src = Source::new("<test>", source);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let typed = TypeInference::infer_program(stmts, src).expect("sema failed");
    let mut air = lower(&typed);
    air = monomorphize(air);
    compute_layouts(&mut air);
    eliminate_copies(&mut air);
    eliminate_dead_locals(&mut air);
    validate_air(&air).expect("AIR should validate");

    let dir = tempdir().expect("tempdir should be created");
    let ll_path = dir.path().join("module.ll");
    let ll_path_str = ll_path.to_string_lossy().to_string();

    let mut codegen = CodegenContext::new("global_no_link");
    codegen
        .compile(&air)
        .expect("codegen compilation should succeed");
    codegen
        .emit_ir(&ll_path_str)
        .expect("llvm ir should be emitted");

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
fn llvm_lowers_const_global_reads_to_real_global_storage() {
    let ir = compile_source_to_verified_ir_without_link(
        r#"
let g = 7

fn main() -> i64 {
    return g
}
"#,
    );
    assert!(ir.contains("@__aelys_global_g = internal global i64 7"), "{ir}");
    assert!(ir.contains("define fastcc i64 @__aelys_main()"), "{ir}");
    assert!(ir.contains("load i64, ptr @__aelys_global_g"), "{ir}");
    assert!(!ir.contains("define i64 @__aelys_main(ptr"), "{ir}");
}

#[test]
fn llvm_rejects_non_constant_enum_global_initializer_cleanly() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
enum Option<T> {
    Some(T),
    None,
}

let g: Option<i64> = Option::Some(7)

fn main() -> i64 {
    return match g {
        Option::Some(v) => v
        Option::None => 0
    }
}
"#,
    )
    .expect("source should be written");

    let err = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, true)
        .expect_err("llvm backend compilation should fail");
    let rendered = err.to_string();
    assert!(
        rendered.contains("global 'g' requires a compile-time constant initializer"),
        "{rendered}"
    );
}

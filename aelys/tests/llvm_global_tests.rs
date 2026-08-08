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
use std::collections::HashSet;
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
    air = monomorphize(air).unwrap();
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

fn lower_optimized_full(source: &str) -> aelys_air::AirProgram {
    let src = Source::new("<test>", source);
    let tokens = Lexer::with_source(src.clone()).scan().expect("lex failed");
    let stmts = Parser::new(tokens, src.clone())
        .parse()
        .expect("parse failed");
    let inference = TypeInference::infer_program_full(
        stmts,
        src,
        HashSet::new(),
        HashSet::from(["print".to_string(), "println".to_string()]),
    )
    .expect("sema failed");
    let mut opt = aelys_opt::Optimizer::new(OptimizationLevel::Standard);
    let checked =
        aelys_air::bir::check(inference.program).unwrap_or_else(|_| panic!("fixture is well-formed"));
    let typed = opt.optimize(checked);
    lower(&typed)
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
    // Aelys-convention main receives an implicit env ptr; check the function exists with fastcc.
    let main_decl = ir
        .lines()
        .find(|l| l.contains("define fastcc i64 @__aelys_main"))
        .expect("__aelys_main must be defined");
    assert!(
        main_decl.contains("fastcc"),
        "__aelys_main must use fastcc:\n{main_decl}"
    );
    assert!(ir.contains("load i64, ptr @__aelys_global_g"), "{ir}");
}

#[test]
fn llvm_lowers_fnptr_global_to_function_symbol() {
    let ir = compile_source_to_verified_ir_without_link(
        r#"
let f: fn() -> i64 = main

fn main() -> i64 {
    return f()
}
"#,
    );
    // Aelys FnPtr globals are fat pointers { fn_ptr, env_ptr }; named fns have null env.
    assert!(
        ir.contains("@__aelys_global_f = internal global { ptr, ptr } { ptr @__aelys_main, ptr null }"),
        "{ir}"
    );
    assert!(ir.contains("load { ptr, ptr }, ptr @__aelys_global_f"), "{ir}");
    assert!(ir.contains("extractvalue { ptr, ptr }"), "{ir}");
    assert!(ir.contains("call fastcc i64 %"), "{ir}");
    assert!(!ir.contains("declare i64 @f()"), "{ir}");
}

#[test]
fn llvm_lowers_fnptr_global_alias_to_same_function_symbol() {
    let ir = compile_source_to_verified_ir_without_link(
        r#"
let g: fn() -> i64 = main
let h: fn() -> i64 = g

fn main() -> i64 {
    return h()
}
"#,
    );
    // Both aliases should resolve to the same fat pointer { fn_ptr, null_env }.
    assert!(
        ir.contains("@__aelys_global_g = internal global { ptr, ptr } { ptr @__aelys_main, ptr null }"),
        "{ir}"
    );
    assert!(
        ir.contains("@__aelys_global_h = internal global { ptr, ptr } { ptr @__aelys_main, ptr null }"),
        "{ir}"
    );
    assert!(ir.contains("load { ptr, ptr }, ptr @__aelys_global_h"), "{ir}");
    assert!(!ir.contains("unknown function 'g'"), "{ir}");
    assert!(!ir.contains("declare i64 @g()"), "{ir}");
}

#[test]
fn llvm_lowers_data_enum_global_alias_to_same_const_aggregate() {
    let ir = compile_source_to_verified_ir_without_link(
        r#"
enum Option<T> {
    Some(T),
    None,
}

let g: Option<i64> = Option::None
let h: Option<i64> = g

fn main() -> i64 {
    return match h {
        Option::Some(v) => v
        Option::None => 9
    }
}
"#,
    );
    assert!(
        ir.contains("@__aelys_global_g = internal global %__aelys_enum___mono_Option_i64 { i32 1"),
        "{ir}"
    );
    assert!(
        ir.contains("@__aelys_global_h = internal global %__aelys_enum___mono_Option_i64 { i32 1"),
        "{ir}"
    );
    assert!(ir.contains("load %__aelys_enum___mono_Option_i64, ptr @__aelys_global_h"), "{ir}");
}

#[test]
fn llvm_lowers_simple_enum_global_to_i32_storage() {
    let ir = compile_source_to_verified_ir_without_link(
        r#"
enum Color {
    Red,
    Green,
}

let c: Color = Color::Green

fn read_color() -> i64 {
    return match c {
        Color::Red => 1
        Color::Green => 2
    }
}
"#,
    );
    assert!(ir.contains("@__aelys_global_c = internal global i32 1"), "{ir}");
    assert!(ir.contains("load i32, ptr @__aelys_global_c"), "{ir}");
}

#[test]
fn lowering_keeps_const_initializer_for_simple_enum_globals_after_optimizer() {
    let air = lower_optimized_full(
        r#"
enum Color {
    Red,
    Green,
}

let c: Color = Color::Green

fn read_color() -> i64 {
    return match c {
        Color::Red => 1
        Color::Green => 2
    }
}
"#,
    );

    let global = air
        .globals
        .iter()
        .find(|global| global.name == "c")
        .expect("global c should exist");
    assert!(matches!(
        global.init,
        Some(aelys_air::AirConst::Int(1, aelys_air::AirIntSize::I32))
    ));
}

#[test]
fn driver_compiles_simple_enum_global_initializer() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
enum Color {
    Red,
    Green,
}

let c: Color = Color::Green

fn read_color() -> i64 {
    return match c {
        Color::Red => 1
        Color::Green => 2
    }
}
"#,
    )
    .expect("source should be written");

    compile_file_with_llvm(&source_path, OptimizationLevel::Standard, true)
        .expect("driver should compile simple enum globals");

    let ir_path = source_path.with_extension("ll");
    let ir = fs::read_to_string(ir_path).expect("llvm ir should be emitted");
    assert!(ir.contains("define fastcc"), "{ir}");
    assert!(ir.contains("ret i64 2"), "{ir}");
}

#[test]
fn llvm_lowers_data_enum_unit_global_to_const_aggregate() {
    let ir = compile_source_to_verified_ir_without_link(
        r#"
enum Option<T> {
    Some(T),
    None,
}

let g: Option<i64> = Option::None

fn read_option() -> i64 {
    return match g {
        Option::Some(v) => v
        Option::None => 33
    }
}
"#,
    );
    assert!(
        ir.contains("@__aelys_global_g = internal global %__aelys_enum___mono_Option_i64 { i32 1"),
        "{ir}"
    );
    assert!(ir.contains("zeroinitializer"), "{ir}");
}

#[test]
fn llvm_lowers_data_enum_payload_global_to_const_aggregate() {
    let ir = compile_source_to_verified_ir_without_link(
        r#"
enum Option<T> {
    Some(T),
    None,
}

let g: Option<i64> = Option::Some(42)

fn read_option() -> i64 {
    return match g {
        Option::Some(v) => v
        Option::None => 0
    }
}
"#,
    );
    assert!(
        ir.contains("@__aelys_global_g = internal global %__aelys_enum___mono_Option_i64 { i32 0"),
        "{ir}"
    );
    assert!(
        ir.contains("[8 x i8] c\"*\\00\\00\\00\\00\\00\\00\\00\"")
            || ir.contains("[8 x i8] [i8 42, i8 0, i8 0, i8 0, i8 0, i8 0, i8 0, i8 0]"),
        "{ir}"
    );
}

#[test]
fn driver_compiles_data_enum_unit_global_initializer() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
enum Option<T> {
    Some(T),
    None,
}

let g: Option<i64> = Option::None

fn read_option() -> i64 {
    return match g {
        Option::Some(v) => v
        Option::None => 33
    }
}
"#,
    )
    .expect("source should be written");

    compile_file_with_llvm(&source_path, OptimizationLevel::Standard, true)
        .expect("driver should compile unit data-enum globals");

    let ir_path = source_path.with_extension("ll");
    let ir = fs::read_to_string(ir_path).expect("llvm ir should be emitted");
    assert!(ir.contains("ret i64 33"), "{ir}");
}

#[test]
fn lowering_keeps_const_initializer_for_payload_enum_globals_after_optimizer() {
    let air = lower_optimized_full(
        r#"
enum Option<T> {
    Some(T),
    None,
}

let g: Option<i64> = Option::Some(7)

fn read_option() -> i64 {
    return match g {
        Option::Some(v) => v
        Option::None => 0
    }
}

"#,
    );

    let global = air
        .globals
        .iter()
        .find(|global| global.name == "g")
        .expect("global g should exist");
    assert!(matches!(
        global.init,
        Some(aelys_air::AirConst::Enum { ref enum_name, tag: 0, ref payload })
            if enum_name == "__mono_Option_i64"
                && matches!(
                    payload.as_slice(),
                    [aelys_air::AirConst::Int(7, aelys_air::AirIntSize::I64)]
                )
    ));
}

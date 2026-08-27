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
use std::process::Command;
use tempfile::tempdir;

fn compile_to_verified_ir(source: &str) -> String {
    compile_to_verified_ir_with_opt(source, OptimizationLevel::None)
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

    let mut codegen = CodegenContext::new("e2e_no_link");
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

fn executable_path_for(source_path: &std::path::Path) -> std::path::PathBuf {
    let mut output = source_path.with_extension("");
    if cfg!(windows) {
        output.set_extension("exe");
    }
    output
}

fn linker_unavailable(error: &str) -> bool {
    error.contains("failed to run `lld-link`: program not found")
        || error.contains("failed to run `link`: program not found")
        || error.contains("failed with status Some(-1073741819)")
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
    let mut i: i64 = 0
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
fn llvm_lowers_println_to_aelys_write_with_slice_abi() {
    let ir = compile_to_verified_ir("fn greet() { println(\"Hello\") }");
    assert!(ir.contains("declare void @__aelys_write(ptr, i64)"), "{ir}");
    assert!(!ir.contains("declare i64 @println"), "{ir}");
    assert!(!ir.contains("declare void @println"), "{ir}");
    assert!(!ir.contains("declare i64 @print"), "{ir}");
    assert!(!ir.contains("declare void @print"), "{ir}");
    assert!(ir.contains("c\"Hello\\00\""), "{ir}");
    assert!(!ir.contains("c\"\\00Hello"), "{ir}");
    assert!(!ir.contains(", i64 0, i64 1"), "{ir}");
    assert!(ir.contains("call void @__aelys_write"), "{ir}");
    assert!(ir.contains("i64 5"), "{ir}");
}

#[test]
fn echo_example() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    let s = "Hello"
    println(s)
    return s.len
}
"#,
    )
    .expect("source should be written");
    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, false) {
        panic!("llvm backend compilation should succeed: {err}");
    }

    let exe_path = executable_path_for(&source_path);
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert!(output.stdout == b"Hello\n" || output.stdout == b"Hello\r\n");
    assert_eq!(output.status.code().unwrap_or(-1), 5);
}

#[test]
fn internal_nul_preserved() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    println("A\0B")
    return 0
}
"#,
    )
    .expect("source should be written");
    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, false) {
        panic!("llvm backend compilation should succeed: {err}");
    }

    let exe_path = executable_path_for(&source_path);
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert!(output.stdout == b"A\0B\n" || output.stdout == b"A\0B\r\n");
}

#[test]
fn len_on_temporary_literal_expression() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    return "hello".len
}
"#,
    )
    .expect("source should be written");
    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, false) {
        panic!("llvm backend compilation should succeed: {err}");
    }

    let exe_path = executable_path_for(&source_path);
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert_eq!(output.status.code().unwrap_or(-1), 5);
}

#[test]
fn len_in_callee_on_string_parameter() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn sink(s: string) -> i64 {
    return s.len
}

fn main() -> i64 {
    return sink("Hello")
}
"#,
    )
    .expect("source should be written");
    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, false) {
        panic!("llvm backend compilation should succeed: {err}");
    }

    let exe_path = executable_path_for(&source_path);
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert_eq!(output.status.code().unwrap_or(-1), 5);
}

#[test]
fn llvm_main_fn_value_uses_backend_symbol_mapping() {
    let ir = compile_source_to_verified_ir_without_link(
        r#"
enum Holder<T> {
    Value(T),
    Empty,
}

fn bounce(h: Holder<fn() -> i64>) -> Holder<fn() -> i64> {
    return h
}

fn main() -> i64 {
    let h: Holder<fn() -> i64> = Holder::Value(main)
    let out = bounce(h)
    return match out {
        Holder::Value(_) => 42
        Holder::Empty => 0
    }
}
"#,
    );
    assert!(ir.contains("@__aelys_main"), "{ir}");
    assert!(
        ir.contains("ptr @__aelys_main") || ir.contains("@__aelys_main"),
        "{ir}"
    );
}

#[test]
fn llvm_rejects_main_with_parameters_for_native_entry() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main(x: i64) -> i64 {
    return x
}
"#,
    )
    .expect("source should be written");
    let err = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, true)
        .expect_err("llvm backend compilation should fail");
    let rendered = err.to_string();
    assert!(
        rendered.contains("invalid native entry: main must have no parameters (found 1)"),
        "{rendered}"
    );
}

#[test]
fn llvm_rejects_main_returning_i32_for_native_entry() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i32 {
    return 1
}
"#,
    )
    .expect("source should be written");
    let err = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, true)
        .expect_err("llvm backend compilation should fail");
    let rendered = err.to_string();
    assert!(
        rendered.contains("type mismatch") || rendered.contains("invalid native entry"),
        "expected type mismatch or native entry error, got: {rendered}"
    );
}

#[test]
fn llvm_preserves_sema_diagnostic_for_return_null_in_i64_function() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn foo() -> i64 {
    for i in 0..10 {
        return null
    }
}
"#,
    )
    .expect("source should be written");
    let err = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, true)
        .expect_err("compilation should fail at sema stage");
    let rendered = err.to_string();
    assert!(
        rendered.contains("expected `i64`, found `null`"),
        "{rendered}"
    );
    assert!(
        !rendered.contains("[llvm-backend]"),
        "sema error must not be re-labeled as llvm backend: {rendered}"
    );
    assert!(
        !rendered.contains(":1:1"),
        "sema error should keep its original source span: {rendered}"
    );
}

#[test]
fn llvm_sema_diagnostic_reports_multiple_errors_separately() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn first() -> i64 {
    return null
}

fn second() -> i64 {
    return null
}
"#,
    )
    .expect("source should be written");
    let err = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, true)
        .expect_err("compilation should fail at sema stage");
    let rendered = err.to_string();
    let error_count = rendered.matches("error[E0301]").count();
    assert!(
        error_count >= 2,
        "should have at least 2 separate error diagnostics, got {}: {rendered}",
        error_count
    );
    assert!(
        rendered.contains("first"),
        "should mention function 'first': {rendered}"
    );
    assert!(
        rendered.contains("second"),
        "should mention function 'second': {rendered}"
    );
}

#[test]
fn llvm_native_entry_maps_negative_i64_main_exit_code_to_u8() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    return -1
}
"#,
    )
    .expect("source should be written");
    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, false) {
        panic!("llvm backend compilation should succeed: {err}");
    }

    let exe_path = executable_path_for(&source_path);
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert_eq!(output.status.code().unwrap_or(-1), 255);
}

#[test]
fn llvm_native_entry_returns_zero_for_void_main() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> void {
    let x: i64 = 1
}
"#,
    )
    .expect("source should be written");
    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, false) {
        panic!("llvm backend compilation should succeed: {err}");
    }

    let exe_path = executable_path_for(&source_path);
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert_eq!(output.status.code().unwrap_or(-1), 0);
}

#[test]
fn llvm_nested_generic_enum_type_arg_compiles() {
    let ir = compile_to_verified_ir(
        r#"
enum Pair<A, B> {
    Both(A, B),
    Neither,
}

enum Boxed<T> {
    Value(T),
    Empty,
}

fn first_from_box(b: Boxed<Pair<i64, string>>) -> i64 {
    return match b {
        Boxed::Value(p) => match p {
            Pair::Both(x, _) => x,
            Pair::Neither => -1,
        },
        Boxed::Empty => -2,
    }
}
"#,
    );
    assert!(
        ir.contains("__mono_Boxed_enum___mono_Pair_i64$str"),
        "nested generic enum monomorphization should survive into IR:\n{ir}"
    );
}

#[test]
fn llvm_nested_generic_enum_unit_variant_only_compiles() {
    let ir = compile_to_verified_ir(
        r#"
enum Pair<A, B> {
    Both(A, B),
    Neither,
}

enum Boxed<T> {
    Value(T),
    Empty,
}

fn get_empty() -> Boxed<Pair<i64, string>> {
    return Boxed::Empty
}
"#,
    );
    assert!(
        ir.contains("__mono_Boxed_enum___mono_Pair_i64$str"),
        "unit-only nested generic enum should still monomorphize parent enum:\n{ir}"
    );
    assert!(
        ir.contains("__mono_Pair_i64$str"),
        "unit-only nested generic enum should also synthesize nested enum def:\n{ir}"
    );
}

#[test]
fn llvm_generic_enum_unit_variant_with_fnptr_type_arg_compiles() {
    let ir = compile_to_verified_ir(
        r#"
enum Holder<T> {
    Value(T),
    Empty,
}

fn apply_default() -> Holder<fn(i64) -> i64> {
    return Holder::Empty
}
"#,
    );
    assert!(
        ir.contains("__mono_Holder_fnptr$i64$Ri64"),
        "fnptr-instantiated generic enum should survive into IR:\n{ir}"
    );
}

#[test]
fn llvm_generic_enum_named_fn_payload_uses_fnptr_mono() {
    let ir = compile_source_to_verified_ir_without_link(
        r#"
enum Holder<T> {
    Value(T),
    Empty,
}

fn inc(x: i64) -> i64 {
    return x + 1
}

fn call_holder(h: Holder<fn(i64) -> i64>) -> i64 {
    return match h {
        Holder::Value(f) => f(41)
        Holder::Empty => 0
    }
}

fn main() -> i64 {
    let h: Holder<fn(i64) -> i64> = Holder::Value(inc)
    return call_holder(h)
}
"#,
    );
    assert!(
        ir.contains("__mono_Holder_fnptr$i64$Ri64"),
        "named function payload enum should use fnptr mono in IR:\n{ir}"
    );
    assert!(
        !ir.contains("__mono_Holder_ptr_void"),
        "named function payload enum must not use ptr_void mono in IR:\n{ir}"
    );
}

#[test]
#[ignore = "multi-module compilation not yet supported by LLVM backend"]
fn llvm_multi_module_strings_compile_and_run() {
    let dir = tempdir().expect("tempdir should be created");
    let module_path = dir.path().join("strings.aelys");
    let source_path = dir.path().join("main.aelys");

    fs::write(
        &module_path,
        r#"
pub fn alpha() -> string {
    return "A"
}

pub fn beta() -> string {
    return "B"
}
"#,
    )
    .expect("module source should be written");

    fs::write(
        &source_path,
        r#"
needs strings

fn main() -> i64 {
    strings.alpha()
    strings.beta()
    return 0
}
"#,
    )
    .expect("main source should be written");

    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, true) {
        if linker_unavailable(&err.to_string()) {
            return;
        }
        panic!("llvm backend compilation should succeed: {err}");
    }

    if !executable_path_for(&source_path).is_file() {
        return;
    }

    let exe_path = executable_path_for(&source_path);
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert!(
        output.stdout.is_empty(),
        "unexpected stdout: {:?}",
        output.stdout
    );
    assert_eq!(output.status.code().unwrap_or(-1), 0);
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

#[test]
fn llvm_string_index_compiles_and_calls_runtime() {
    let ir = compile_to_verified_ir(
        r#"
fn char_at(s: string, i: i64) -> string {
    return s[i]
}
"#,
    );
    assert!(
        ir.contains("@__aelys_str_char_at"),
        "string index should call __aelys_str_char_at:\n{ir}"
    );
    let char_at_decl = ir
        .lines()
        .find(|l| l.contains("declare") && l.contains("@__aelys_str_char_at"))
        .expect("__aelys_str_char_at must be declared");
    assert!(
        char_at_decl.contains("__aelys_string"),
        "char_at must involve %__aelys_string type:\n{char_at_decl}"
    );
    assert!(
        char_at_decl.contains("ptr") && char_at_decl.contains("i64"),
        "char_at must accept (ptr, i64, i64) args:\n{char_at_decl}"
    );
}

#[test]
fn llvm_array_literal_index_read_compiles() {
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
        "array literal should use alloca [3 x i64]:\n{ir}"
    );
    assert!(
        ir.contains("store i64 10"),
        "array literal should store first element:\n{ir}"
    );
    assert!(
        ir.contains("store i64 20"),
        "array literal should store second element:\n{ir}"
    );
    assert!(
        ir.contains("store i64 30"),
        "array literal should store third element:\n{ir}"
    );
    assert!(
        ir.contains("getelementptr"),
        "array index should generate GEP:\n{ir}"
    );
    assert!(
        ir.contains("@__aelys_panic"),
        "array index should panic on OOB:\n{ir}"
    );
}

#[test]
fn llvm_array_index_write_compiles() {
    let ir = compile_to_verified_ir(
        r#"
fn mutate() -> i64 {
    let mut arr = [10, 20, 30]
    arr[0] = 99
    return arr[0]
}
"#,
    );
    assert!(
        ir.contains("alloca [3 x i64]"),
        "array should use stack allocation:\n{ir}"
    );
    assert!(
        ir.contains("store i64 99"),
        "array index write should generate store for new value:\n{ir}"
    );
    assert!(
        ir.contains("getelementptr"),
        "array index write should generate GEP:\n{ir}"
    );
}

#[test]
fn llvm_index_in_function_argument_compiles() {
    let ir = compile_to_verified_ir(
        r#"
fn identity(x: i64) -> i64 { return x }
fn use_index() -> i64 {
    let arr = [10, 20, 30]
    return identity(arr[1])
}
"#,
    );
    assert!(
        ir.contains("@identity"),
        "should call identity function:\n{ir}"
    );
    assert!(
        ir.contains("getelementptr"),
        "index in arg position should generate GEP:\n{ir}"
    );
}

/// array index with variable (not constant) generates bounds check.
#[test]
fn llvm_array_index_with_variable_has_bounds_check() {
    let ir = compile_to_verified_ir(
        r#"
fn at(arr: [i64; 3], i: i64) -> i64 {
    return arr[i]
}
"#,
    );
    assert!(
        ir.contains("icmp uge"),
        "variable index should have bounds check:\n{ir}"
    );
    assert!(
        ir.contains("@__aelys_panic"),
        "variable index should panic on OOB:\n{ir}"
    );
    assert!(ir.contains("idx_oob:"), "should have idx_oob block:\n{ir}");
    assert!(ir.contains("idx_ok:"), "should have idx_ok block:\n{ir}");
}

#[test]
fn llvm_array_swap_pattern_compiles() {
    let ir = compile_to_verified_ir(
        r#"
fn swap(mut arr: [i64; 3], i: i64, j: i64) -> void {
    let tmp = arr[i]
    arr[i] = arr[j]
    arr[j] = tmp
}
"#,
    );
    let gep_count = ir.matches("getelementptr").count();
    assert!(
        gep_count >= 3,
        "swap should generate at least 3 GEPs (read i, read j, write i, write j), got {gep_count}:\n{ir}"
    );
    let store_count = ir.matches("store i64").count();
    assert!(
        store_count >= 2,
        "swap should generate at least 2 stores, got {store_count}:\n{ir}"
    );
    assert!(
        ir.contains("ret void"),
        "void function with index assignment should emit ret void:\n{ir}"
    );
}

#[test]
fn llvm_string_literal_index_compiles() {
    let ir = compile_to_verified_ir(
        r#"
fn first_char() -> string {
    let s = "hello"
    return s[0]
}
"#,
    );
    assert!(
        ir.contains("@__aelys_str_char_at"),
        "string literal index should call __aelys_str_char_at:\n{ir}"
    );
}

#[test]
fn llvm_loop_with_array_index_compiles() {
    let ir = compile_to_verified_ir(
        r#"
fn sum_array(arr: [i64; 10], n: i64) -> i64 {
    let mut total: i64 = 0
    let mut i: i64 = 0
    while i < n {
        total = total + arr[i]
        i = i + 1
    }
    return total
}
"#,
    );
    assert!(has_back_edge(&ir), "loop should have a back edge:\n{ir}");
    assert!(
        ir.contains("getelementptr"),
        "loop body index should generate GEP:\n{ir}"
    );
    assert!(
        ir.contains("icmp uge"),
        "loop body index should have bounds check:\n{ir}"
    );
}

#[test]
fn llvm_void_function_with_assignment_produces_ret_void() {
    let ir = compile_to_verified_ir(
        r#"
fn set_it(x: i64) -> void {
    let mut y: i64 = 0
    y = x
}
"#,
    );
    assert!(
        ir.contains("ret void"),
        "void function with assignment should emit ret void:\n{ir}"
    );
    assert!(
        !ir.contains("ret ptr null"),
        "void function must not emit ret ptr null:\n{ir}"
    );
}

#[test]
fn llvm_void_function_with_index_assign_produces_ret_void() {
    let ir = compile_to_verified_ir(
        r#"
fn fill(mut arr: [i64; 3], i: i64, val: i64) -> void {
    arr[i] = val
}
"#,
    );
    assert!(
        ir.contains("ret void"),
        "void function with index assign should emit ret void:\n{ir}"
    );
    assert!(
        !ir.contains("ret ptr null"),
        "void function must not emit ret ptr null:\n{ir}"
    );
}

#[test]
fn llvm_int_div_emits_div_zero_check() {
    let ir = compile_to_verified_ir(
        r#"
fn divide(a: i64, b: i64) -> i64 {
    return a / b
}
"#,
    );
    assert!(
        ir.contains("icmp eq"),
        "integer div should compare divisor to zero:\n{ir}"
    );
    assert!(
        ir.contains("div_zero:"),
        "should have div_zero trap block:\n{ir}"
    );
    assert!(
        ir.contains("div_ok:"),
        "should have div_ok continuation block:\n{ir}"
    );
    assert!(
        ir.contains("@__aelys_panic"),
        "div-by-zero should call __aelys_panic:\n{ir}"
    );
}

#[test]
fn llvm_int_rem_emits_div_zero_check() {
    let ir = compile_to_verified_ir(
        r#"
fn modulo(a: i64, b: i64) -> i64 {
    return a % b
}
"#,
    );
    assert!(
        ir.contains("div_zero:"),
        "integer rem should have div_zero trap block:\n{ir}"
    );
    assert!(
        ir.contains("@__aelys_panic"),
        "integer rem should call __aelys_panic:\n{ir}"
    );
}

#[test]
fn llvm_unsigned_div_emits_div_zero_check() {
    let ir = compile_to_verified_ir(
        r#"
fn udivide(a: u64, b: u64) -> u64 {
    return a / b
}
"#,
    );
    assert!(
        ir.contains("div_zero:"),
        "unsigned div should have div_zero trap block:\n{ir}"
    );
    assert!(
        ir.contains("udiv"),
        "unsigned div should emit udiv instruction:\n{ir}"
    );
}

#[test]
fn llvm_float_div_has_no_div_zero_check() {
    let ir = compile_to_verified_ir(
        r#"
fn fdivide(a: f64, b: f64) -> f64 {
    return a / b
}
"#,
    );
    assert!(
        !ir.contains("div_zero:"),
        "float div should NOT have div_zero check:\n{ir}"
    );
    assert!(
        ir.contains("fdiv"),
        "float div should emit fdiv instruction:\n{ir}"
    );
}

/// div-zero check survives -o2 the panic call must not be optimized away.
#[test]
fn llvm_div_zero_check_survives_o2() {
    let ir = compile_to_verified_ir_with_opt(
        r#"
fn divide(a: i64, b: i64) -> i64 {
    return a / b
}
"#,
        OptimizationLevel::Standard,
    );
    assert!(
        ir.contains("@__aelys_panic"),
        "div-zero check must survive -O2 ,  panic must not be eliminated:\n{ir}"
    );
}

#[test]
fn llvm_div_by_zero_runtime_panics() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    let a: i64 = 42
    let b: i64 = 0
    return a / b
}
"#,
    )
    .expect("source should be written");
    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
        panic!("llvm backend compilation should succeed: {err}");
    }

    let exe_path = executable_path_for(&source_path);
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert!(
        !output.status.success(),
        "division by zero should cause non-zero exit"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("division by zero"),
        "panic message should contain 'division by zero', got: {stderr}"
    );
}

#[test]
fn llvm_div_by_zero_runtime_panics_at_o2() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    let a: i64 = 42
    let b: i64 = 0
    return a / b
}
"#,
    )
    .expect("source should be written");
    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::Standard, false) {
        panic!("llvm backend compilation should succeed: {err}");
    }

    let exe_path = executable_path_for(&source_path);
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert!(
        !output.status.success(),
        "division by zero at -O2 should cause non-zero exit"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("division by zero"),
        "-O2 should keep the panic, not merely exit non-zero, got: {stderr}"
    );
}

#[test]
fn llvm_rem_by_zero_runtime_panics() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    let a: i64 = 42
    let b: i64 = 0
    return a % b
}
"#,
    )
    .expect("source should be written");
    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
        panic!("llvm backend compilation should succeed: {err}");
    }

    let exe_path = executable_path_for(&source_path);
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert!(
        !output.status.success(),
        "modulo by zero should cause non-zero exit"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("division by zero"),
        "modulo by zero should panic, not merely exit non-zero, got: {stderr}"
    );
}

#[test]
fn llvm_normal_div_still_works() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    let a: i64 = 100
    let b: i64 = 4
    return a / b
}
"#,
    )
    .expect("source should be written");
    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
        panic!("llvm backend compilation should succeed: {err}");
    }

    let exe_path = executable_path_for(&source_path);
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert_eq!(
        output.status.code().unwrap_or(-1),
        25,
        "100 / 4 should return 25"
    );
}

#[test]
fn llvm_div_by_zero_in_loop_panics() {
    let dir = tempdir().expect("tempdir should be created");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    let mut sum: i64 = 0
    for i in 0..3 {
        let divisor: i64 = 2 - i
        sum = sum + 10 / divisor
    }
    return sum
}
"#,
    )
    .expect("source should be written");
    if let Err(err) = compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
        panic!("llvm backend compilation should succeed: {err}");
    }

    let exe_path = executable_path_for(&source_path);
    assert!(
        exe_path.is_file(),
        "native executable should be produced at {}",
        exe_path.display()
    );
    let output = Command::new(&exe_path)
        .output()
        .expect("compiled executable should run");
    assert!(
        !output.status.success(),
        "division by zero in loop iteration should panic"
    );
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("division by zero"),
        "the loop must reach the zero divisor and panic, not return a sum, got: {stderr}"
    );
}

use aelys_cli::cli::commands::run::run_with_options;
use aelys_common::WarningConfig;
use aelys_opt::OptimizationLevel;

#[test]
fn run_accepts_bytecode_with_magic() {
    let dir = std::env::temp_dir().join("aelys_cli_run_bytecode");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let bytecode_path = dir.join("program.avbc");

    let mut function = aelys_bytecode::Function::new(None, 0);
    function.constants.push(aelys_bytecode::Value::int(2));
    function.num_registers = 1;
    function.emit_b(aelys_bytecode::OpCode::LoadK, 0, 0, 1);
    function.emit_a(aelys_bytecode::OpCode::Return, 0, 0, 0, 1);
    function.finalize_bytecode();

    let heap = aelys_bytecode::Heap::new();
    let bytes = aelys_bytecode::asm::serialize(&function, &heap);
    std::fs::write(&bytecode_path, bytes).unwrap();

    let result = run_with_options(
        bytecode_path.to_str().unwrap(),
        Vec::new(),
        Vec::new(),
        OptimizationLevel::Standard,
        WarningConfig::new(),
    );

    assert!(result.is_ok());
}

#[test]
fn run_bytecode_registers_stdlib_globals() {
    let dir = std::env::temp_dir().join("aelys_cli_run_stdlib");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("hello.aelys");
    std::fs::write(&src_path, "needs std.io\nio.print(\"hi\")\n").unwrap();

    let bytecode_path =
        aelys_cli::cli::commands::compile::compile_to_avbc(&src_path, OptimizationLevel::None)
            .unwrap();

    let result = run_with_options(
        bytecode_path.to_str().unwrap(),
        Vec::new(),
        Vec::new(),
        OptimizationLevel::Standard,
        WarningConfig::new(),
    );

    assert!(result.is_ok());
}

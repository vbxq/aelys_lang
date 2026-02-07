use aelys_cli::cli::commands::compile::compile_to_avbc;
use aelys_opt::OptimizationLevel;

#[test]
fn compile_writes_avbc() {
    let dir = std::env::temp_dir().join("aelys_cli_compile_test");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("main.aelys");
    std::fs::write(&src_path, "let x = 1\n").unwrap();

    let output = compile_to_avbc(&src_path, OptimizationLevel::None).unwrap();

    assert!(output.exists());
    assert_eq!(output.extension().unwrap(), "avbc");
}

use aelys_cli::cli::commands::run::run_with_options;
use aelys_opt::OptimizationLevel;

#[test]
fn run_rejects_invalid_vm_args() {
    let err = run_with_options(
        "missing.aelys",
        Vec::new(),
        vec!["-ae.max-heap=1".to_string()],
        OptimizationLevel::None,
    )
    .unwrap_err();

    assert!(err.contains("invalid value for"));
}

#[test]
fn run_accepts_aasm_file() {
    let dir = std::env::temp_dir().join("aelys_cli_run_aasm");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let aasm_path = dir.join("program.aasm");
    std::fs::write(
        &aasm_path,
        ".version 1\n.function 0\n  .arity 0\n  .registers 1\n  .constants\n    0: int 2\n  .code\n    0000: LoadK r0, 0\n    0001: Return r0\n",
    )
    .unwrap();

    let result = run_with_options(
        aasm_path.to_str().unwrap(),
        Vec::new(),
        Vec::new(),
        OptimizationLevel::Standard,
    );

    assert!(result.is_ok());
}

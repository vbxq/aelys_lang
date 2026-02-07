use aelys_cli::cli::run_with_args;

#[test]
fn compile_rejects_vm_args() {
    let args = vec![
        "aelys".to_string(),
        "compile".to_string(),
        "main.aelys".to_string(),
        "-ae.trusted=true".to_string(),
    ];

    let err = run_with_args(&args).unwrap_err();
    assert!(err.contains("vm flags are only supported for run or repl"));
}

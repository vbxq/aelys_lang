use aelys_cli::cli::commands::repl::run_repl_with_io;
use aelys_opt::OptimizationLevel;

#[test]
fn repl_executes_input_and_exits() {
    let input = "1 + 1\nexit\n";
    let mut output = Vec::new();

    run_repl_with_io(
        input.as_bytes(),
        &mut output,
        OptimizationLevel::None,
        Vec::new(),
    )
    .unwrap();

    let text = String::from_utf8(output).unwrap();
    assert!(text.contains("2"));
}

#[test]
fn repl_recovers_after_error() {
    let input = "1 +\n1 + 1\nexit\n";
    let mut output = Vec::new();

    run_repl_with_io(
        input.as_bytes(),
        &mut output,
        OptimizationLevel::None,
        Vec::new(),
    )
    .unwrap();

    let text = String::from_utf8(output).unwrap();
    assert!(text.to_lowercase().contains("error"));
    assert!(text.contains("2"));
}

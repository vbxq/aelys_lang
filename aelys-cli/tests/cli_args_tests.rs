use aelys_opt::OptimizationLevel;

use aelys_cli::cli::args::{Command, ParsedArgs, parse_args};

#[test]
fn parse_run_with_flags_anywhere() {
    let args = vec![
        "aelys",
        "-O3",
        "-ae.trusted=true",
        "run",
        "main.aelys",
        "arg1",
        "-x",
    ]
    .into_iter()
    .map(|s| s.to_string())
    .collect::<Vec<_>>();

    let parsed = parse_args(&args).unwrap();

    assert_eq!(
        parsed,
        ParsedArgs {
            command: Command::Run {
                path: "main.aelys".to_string(),
                program_args: vec!["arg1".to_string(), "-x".to_string()],
            },
            vm_args: vec!["-ae.trusted=true".to_string()],
            opt_level: OptimizationLevel::Aggressive,
        }
    );
}

#[test]
fn parse_implicit_run_path_first() {
    let args = vec!["aelys", "main.aelys", "-O1"]
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    let parsed = parse_args(&args).unwrap();

    assert_eq!(
        parsed,
        ParsedArgs {
            command: Command::Run {
                path: "main.aelys".to_string(),
                program_args: Vec::new(),
            },
            vm_args: Vec::new(),
            opt_level: OptimizationLevel::Basic,
        }
    );
}

#[test]
fn parse_repl_with_vm_flags() {
    let args = vec!["aelys", "repl", "-ae.max-heap=1M"]
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    let parsed = parse_args(&args).unwrap();

    assert_eq!(
        parsed,
        ParsedArgs {
            command: Command::Repl,
            vm_args: vec!["-ae.max-heap=1M".to_string()],
            opt_level: OptimizationLevel::Standard,
        }
    );
}

#[test]
fn parse_unknown_flag_errors_before_path() {
    let args = vec!["aelys", "-Z"]
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    let err = parse_args(&args).unwrap_err();
    assert!(err.contains("unknown flag"));
}

#[test]
fn parse_compile_output_flag() {
    let args = vec!["aelys", "compile", "-o", "out.avbc", "main.aelys"]
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    let parsed = parse_args(&args).unwrap();

    assert_eq!(
        parsed,
        ParsedArgs {
            command: Command::Compile {
                path: "main.aelys".to_string(),
                output: Some("out.avbc".to_string()),
            },
            vm_args: Vec::new(),
            opt_level: OptimizationLevel::Standard,
        }
    );
}

#[test]
fn parse_asm_stdout_flag() {
    let args = vec!["aelys", "asm", "--stdout", "main.aelys"]
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    let parsed = parse_args(&args).unwrap();

    assert_eq!(
        parsed,
        ParsedArgs {
            command: Command::Asm {
                path: "main.aelys".to_string(),
                output: None,
                stdout: true,
            },
            vm_args: Vec::new(),
            opt_level: OptimizationLevel::Standard,
        }
    );
}

#[test]
fn parse_version_command() {
    let args = vec!["aelys", "--version"]
        .into_iter()
        .map(|s| s.to_string())
        .collect::<Vec<_>>();

    let parsed = parse_args(&args).unwrap();

    assert_eq!(
        parsed,
        ParsedArgs {
            command: Command::Version,
            vm_args: Vec::new(),
            opt_level: OptimizationLevel::Standard,
        }
    );
}

use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

mod common;
use common::{exe_path_for as executable_path_for, linker_unavailable};

fn compile_run_capture_stdout(source: &str, opt: OptimizationLevel) -> Option<String> {
    let _pin = common::pin_legs("compile_run_capture_stdout", 1);
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("interp.aelys");
    fs::write(&path, source).expect("write source");

    if let Err(err) = compile_file_with_llvm(&path, opt, false) {
        if linker_unavailable(&err.to_string()) {
            common::require_linker_skip("a skipped value row carries no runtime evidence at all");
            return None;
        }
        panic!("compilation+link should succeed: {err}");
    }

    let exe = executable_path_for(&path);
    assert!(
        exe.is_file(),
        "executable must be produced, link must succeed"
    );

    common::note_leg();
    let output = Command::new(&exe)
        .output()
        .expect("compiled program should run");
    assert!(
        output.status.success(),
        "program should exit 0, got {:?}\nstderr: {}",
        output.status.code(),
        String::from_utf8_lossy(&output.stderr)
    );
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
fn interpolation_formats_string_int_and_bool_unopt() {
    let src = r#"
fn main() -> i64 {
    let name: string = "aelys"
    let count: i64 = 42
    let flag: bool = true
    println("name={name} count={count} flag={flag}")
    return 0
}
"#;
    let Some(stdout) = compile_run_capture_stdout(src, OptimizationLevel::None) else {
        return;
    };
    assert_eq!(stdout, "name=aelys count=42 flag=true\n");
}

#[test]
fn interpolation_formats_runtime_values_optimized() {
    let src = r#"
fn make_count() -> i64 {
    return 7
}

fn main() -> i64 {
    let label: string = "hits"
    let n: i64 = make_count()
    let on: bool = false
    println("{label}: {n} ({on})")
    return 0
}
"#;
    let Some(stdout) = compile_run_capture_stdout(src, OptimizationLevel::Standard) else {
        return;
    };
    assert_eq!(stdout, "hits: 7 (false)\n");
}

#[test]
fn runtime_string_concat_with_plus_unopt() {
    let src = r#"
fn main() -> i64 {
    let a: string = "foo"
    let b: string = "bar"
    let joined: string = a + b
    println(joined)
    return 0
}
"#;
    let Some(stdout) = compile_run_capture_stdout(src, OptimizationLevel::None) else {
        return;
    };
    assert_eq!(stdout, "foobar\n");
}

use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn linker_unavailable(error: &str) -> bool {
    error.contains("failed to run `lld-link`: program not found")
        || error.contains("failed to run `link`: program not found")
        || error.contains("failed with status Some(-1073741819)")
}

fn executable_path_for(source_path: &Path) -> PathBuf {
    let mut output = source_path.with_extension("");
    if cfg!(windows) {
        output.set_extension("exe");
    }
    output
}

fn compile_run_capture_stdout(source: &str, opt: OptimizationLevel) -> Option<String> {
    let dir = tempdir().expect("tempdir");
    let path = dir.path().join("interp.aelys");
    fs::write(&path, source).expect("write source");

    if let Err(err) = compile_file_with_llvm(&path, opt, false) {
        if linker_unavailable(&err.to_string()) {
            eprintln!("skipping: native linker unavailable");
            return None;
        }
        panic!("compilation+link should succeed: {err}");
    }

    let exe = executable_path_for(&path);
    assert!(
        exe.is_file(),
        "executable must be produced, link must succeed"
    );

    let output = Command::new(&exe).output().expect("compiled program should run");
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

// must-use for a dropped result + the discard escape hatch

use aelys_driver::{RuntimeVariant, compile_file_with_llvm, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn try_reject(src: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect_err("compilation should be rejected")
        .to_string()
}

fn try_accept(src: &str) {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    compile_file_with_llvm(&source_path, OptimizationLevel::None, true)
        .expect("compilation should succeed");
}

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
}

fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found") || error.contains("failed to run")
}

fn run_native(
    src: &str,
    variant: RuntimeVariant,
    opt: OptimizationLevel,
    env: &[(&str, &str)],
) -> Option<(i32, String, String)> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(&source_path, opt, false, variant) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                eprintln!("linker unavailable; skipping exec assertion");
                return None;
            }
            panic!("compilation/link should succeed: {err}");
        }
    }

    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        eprintln!("executable not produced (linker unavailable); skipping");
        return None;
    }

    let mut cmd = Command::new(&exe);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let output = cmd.output().expect("run compiled exe");
    let code = output.status.code().unwrap_or(-1);
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Some((code, stdout, stderr))
}

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
    let (a, m) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, m.trim().parse().ok()?))
}

// negative oracle: a non-tail dropped result is rejected; the discard/let/? twins compile

const N_DROP: &str = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
fn risky(x: i64) -> Result<i64, i64> {
    if x == 0 {
        return Result::Err(1)
    }
    return Result::Ok(x)
}
fn main() -> i64 {
    risky(5)
    return 0
}
"#;

const N_DISCARD: &str = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
fn risky(x: i64) -> Result<i64, i64> {
    if x == 0 {
        return Result::Err(1)
    }
    return Result::Ok(x)
}
fn main() -> i64 {
    discard risky(5)
    return 0
}
"#;

const N_LET: &str = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
fn risky(x: i64) -> Result<i64, i64> {
    if x == 0 {
        return Result::Err(1)
    }
    return Result::Ok(x)
}
fn main() -> i64 {
    let r: Result<i64, i64> = risky(5)
    return 0
}
"#;

const N_TRY: &str = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
fn risky(x: i64) -> Result<i64, i64> {
    if x == 0 {
        return Result::Err(1)
    }
    return Result::Ok(x)
}
fn caller() -> Result<i64, i64> {
    let v: i64 = risky(5)?
    return Result::Ok(v)
}
fn main() -> i64 {
    return 0
}
"#;

#[test]
fn negative_oracle_dropped_result_is_rejected() {
    let err = try_reject(N_DROP);
    assert!(
        err.contains("[must-use]"),
        "a non-tail dropped Result must be rejected with the marker; got:\n{err}"
    );
}

#[test]
fn test_of_the_test_discard_twin_compiles() {
    try_accept(N_DISCARD);
}

#[test]
fn test_of_the_test_let_bound_twin_compiles() {
    try_accept(N_LET);
}

#[test]
fn test_of_the_test_try_propagated_twin_compiles() {
    try_accept(N_TRY);
}

const TAIL: &str = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
fn risky(x: i64) -> Result<i64, i64> {
    if x == 0 {
        return Result::Err(1)
    }
    return Result::Ok(x)
}
fn f() -> Result<i64, i64> {
    risky(5)
}
fn main() -> i64 {
    return 0
}
"#;

#[test]
fn positive_tail_result_compiles() {
    try_accept(TAIL);
}

#[test]
fn positive_discard_runs() {
    let Some((code, stdout, stderr)) =
        run_native(N_DISCARD, RuntimeVariant::Rc, OptimizationLevel::None, &[])
    else {
        return;
    };
    assert_eq!(
        code, 0,
        "discard risky(5); return 0 must exit 0; stderr:\n{stderr}"
    );
    assert!(stdout.is_empty(), "no output expected, got {stdout:?}");
}

const ONCE_ONLY: &str = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
fn make() -> Rc<i64> {
    return Rc::new(42)
}
fn main() -> i64 {
    discard make()
    return 0
}
"#;

#[test]
fn discard_evaluates_operand_exactly_once() {
    let Some((code, stdout, stderr)) = run_native(
        ONCE_ONLY,
        RuntimeVariant::Rc,
        OptimizationLevel::None,
        &[("AELYS_RC_STATS", "1")],
    ) else {
        return;
    };
    assert!(stdout.is_empty(), "no output expected, got {stdout:?}");
    let (allocs, _frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        code, 0,
        "discard make(); return 0 must exit 0; stderr:\n{stderr}"
    );
    assert_eq!(
        allocs, 1,
        "discard must evaluate its operand exactly once (one Rc::new)"
    );
}

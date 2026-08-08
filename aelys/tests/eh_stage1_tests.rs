
use aelys_driver::{compile_file_with_llvm, compile_file_with_llvm_variant, RuntimeVariant};
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

const BEHAVIORAL_SRC: &str = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}

enum Option<T> {
    Some(T),
    None,
}

fn step_a(x: i64) -> Result<i64, i64> {
    if x == 0 {
        return Result::Err(7)
    }
    return Result::Ok(x + 1)
}

fn step_b(x: i64) -> Result<i64, i64> {
    if x > 100 {
        return Result::Err(9)
    }
    return Result::Ok(x * 2)
}

fn load_mesh(x: i64) -> Result<i64, i64> {
    let a: i64 = step_a(x)?
    let b: i64 = step_b(a)?
    return Result::Ok(b)
}

fn first_positive(x: i64) -> Option<i64> {
    if x > 0 {
        return Option::Some(x)
    }
    return Option::None
}

fn chain_opt(x: i64) -> Option<i64> {
    let v: i64 = first_positive(x)?
    return Option::Some(v + 100)
}

fn main() -> i64 {
    let happy: i64 = match load_mesh(10) {
        Result::Ok(v) => v
        Result::Err(e) => e
    }
    let sad: i64 = match load_mesh(0) {
        Result::Ok(v) => v
        Result::Err(e) => e
    }
    let opt_some: i64 = match chain_opt(5) {
        Option::Some(v) => v
        Option::None => 0
    }
    let opt_none: i64 = match chain_opt(0) {
        Option::Some(v) => v
        Option::None => 1
    }
    return happy + sad + opt_some + opt_none
}
"#;

#[test]
fn behavioral_chained_try_and_option_propagation() {
    let Some((code, stdout, stderr)) =
        run_native(BEHAVIORAL_SRC, RuntimeVariant::Rc, OptimizationLevel::None, &[])
    else {
        return;
    };
    assert_eq!(
        code, 135,
        "chained `?` (Ok path) + propagated Err + Option None must sum to 135; stderr:\n{stderr}"
    );
    assert!(stdout.is_empty(), "no output expected, got {stdout:?}");
}

// negative oracle: cross-error-type `?` is rejected; the matching-type twin compiles

const NEG_MISMATCH_SRC: &str = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
enum Fault {
    Bad,
}
fn inner() -> Result<i64, Fault> {
    return Result::Err(Fault::Bad)
}
fn outer() -> Result<i64, i64> {
    let v: i64 = inner()?
    return Result::Ok(v)
}
fn main() -> i64 {
    return 0
}
"#;

const NEG_MATCHING_SRC: &str = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}
fn inner() -> Result<i64, i64> {
    return Result::Err(5)
}
fn outer() -> Result<i64, i64> {
    let v: i64 = inner()?
    return Result::Ok(v)
}
fn main() -> i64 {
    return 0
}
"#;

#[test]
fn negative_oracle_cross_error_type_is_rejected() {
    let err = try_reject(NEG_MISMATCH_SRC);
    assert!(
        err.contains("[?-stage1]"),
        "cross-error-type `?` (Fault vs i64) must be rejected with the branded marker; got:\n{err}"
    );
}

#[test]
fn negative_oracle_matching_error_type_compiles() {
    try_accept(NEG_MATCHING_SRC);
}

const SEAM_SRC: &str = r#"
enum Result<T, E> {
    Ok(T),
    Err(E),
}

fn may_fail(x: i64) -> Result<i64, i64> {
    if x == 0 {
        return Result::Err(3)
    }
    return Result::Ok(x)
}

fn with_rc(x: i64) -> Result<i64, i64> {
    let node: Rc<i64> = Rc::new(99)
    let y: i64 = may_fail(x)?
    let got: i64 = Rc::get(node)
    return Result::Ok(y + got)
}

fn main() -> i64 {
    let a: i64 = match with_rc(0) {
        Result::Ok(v) => v
        Result::Err(e) => e
    }
    let b: i64 = match with_rc(5) {
        Result::Ok(v) => v
        Result::Err(e) => e
    }
    return a + b
}
"#;

fn run_seam(opt: OptimizationLevel) -> Option<(i32, i64, i64)> {
    let (code, stdout, stderr) = run_native(
        SEAM_SRC,
        RuntimeVariant::Rc,
        opt,
        &[("AELYS_RC_STATS", "1")],
    )?;
    assert!(stdout.is_empty(), "no output expected, got {stdout:?}");
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    Some((code, allocs, frees))
}

#[test]
fn seam_rc_live_across_try_early_return_o0() {
    let Some((code, allocs, frees)) = run_seam(OptimizationLevel::None) else {
        return;
    };
    assert_eq!(code, 107, "Rc live across a `?` early-return: 3 + 104 = 107 at -O0");
    assert_eq!(allocs, 2, "two Rc::new allocations at -O0 (allocs={allocs})");
    assert_eq!(
        allocs, frees,
        "no leak, no double-free across the `?` early-return at -O0 (allocs={allocs} frees={frees})"
    );
}

#[test]
fn seam_rc_live_across_try_early_return_o2() {
    let Some((code, allocs, frees)) = run_seam(OptimizationLevel::Standard) else {
        return;
    };
    assert_eq!(code, 107, "3 + 104 = 107 at -O2");
    assert_eq!(allocs, 2, "two Rc::new allocations at -O2 (allocs={allocs})");
    assert_eq!(
        allocs, frees,
        "balanced across the `?` early-return at -O2 (allocs={allocs} frees={frees})"
    );
}

#[test]
fn seam_rc_live_across_try_early_return_o3() {
    let Some((code, allocs, frees)) = run_seam(OptimizationLevel::Aggressive) else {
        return;
    };
    assert_eq!(code, 107, "3 + 104 = 107 at -O3");
    assert_eq!(allocs, 2, "two Rc::new allocations at -O3 (allocs={allocs})");
    assert_eq!(
        allocs, frees,
        "balanced across the `?` early-return at -O3 (allocs={allocs} frees={frees})"
    );
}


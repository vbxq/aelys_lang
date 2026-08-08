use aelys_driver::{compile_file_with_llvm_variant, lower_file_to_air, RuntimeVariant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

const RESOURCE: &str = "struct Resource { id: i64 }\n";

fn reject(body: &str) -> String {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, format!("{RESOURCE}{body}")).expect("write source");
    match lower_file_to_air(&source_path, OptimizationLevel::None) {
        Ok(_) => panic!("expected the program to be rejected by the move checker, but it compiled"),
        Err(err) => err,
    }
}

fn accepts(body: &str) {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, format!("{RESOURCE}{body}")).expect("write source");
    lower_file_to_air(&source_path, OptimizationLevel::None)
        .unwrap_or_else(|err| panic!("the move checker must accept this program: {err}"));
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

fn run_stdout(body: &str) -> Option<(i32, String)> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, format!("{RESOURCE}{body}")).expect("write source");

    match compile_file_with_llvm_variant(&source_path, OptimizationLevel::None, false, RuntimeVariant::Rc) {
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

    let output = Command::new(&exe).output().expect("run compiled exe");
    let code = output.status.code().expect("exit code");
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    Some((code, stdout))
}

fn count_lines(stdout: &str, needle: &str) -> usize {
    stdout.lines().filter(|l| l.trim() == needle).count()
}

// -------- compile-fail: each must be rejected (a permissive checker would accept them) ------

#[test]
fn use_after_move_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let a = Resource{id: 1}
    let b = a
    println(a.id)
    return 0
}
"#,
    );
    assert!(err.contains("[move]"), "must carry the move marker: {err}");
    assert!(
        err.contains("after it was moved"),
        "must be a use-after-move diagnostic: {err}"
    );
}

#[test]
fn double_move_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let a = Resource{id: 1}
    let b = a
    let c = a
    return 0
}
"#,
    );
    assert!(err.contains("[move]"), "must carry the move marker: {err}");
    assert!(
        err.contains("already moved"),
        "must be a double-move diagnostic: {err}"
    );
}

#[test]
fn use_of_maybe_moved_is_rejected() {
    let err = reject(
        r#"
fn main() -> i64 {
    let cond = true
    let a = Resource{id: 1}
    if cond {
        let b = a
    }
    println(a.id)
    return 0
}
"#,
    );
    assert!(err.contains("[move]"), "must carry the move marker: {err}");
    assert!(
        err.contains("may have been moved") || err.contains("earlier branch"),
        "must be a use-of-maybe-moved diagnostic: {err}"
    );
}

#[test]
fn affine_deterministic_destruction_no_drop_of_moved_source() {
    accepts(
        r#"
fn main() -> i64 {
    let a = Resource{id: 3}
    let b = a
    return 0
}
"#,
    );
    let Some((code, out)) = run_stdout(
        r#"
fn main() -> i64 {
    let a = Resource{id: 3}
    let b = a
    return 0
}
"#,
    ) else {
        return;
    };
    assert_eq!(code, 0, "program should exit 0; stdout:\n{out}");
    assert_eq!(
        count_lines(&out, "3"),
        1,
        "a moved-out source must not be dropped: expected exactly one `3`; stdout:\n{out}"
    );
}

#[test]
fn move_then_reassign_runs() {
    let Some((code, out)) = run_stdout(
        r#"
fn main() -> i64 {
    let mut a = Resource{id: 1}
    let b = a
    a = Resource{id: 2}
    println(a.id)
    return 0
}
"#,
    ) else {
        return;
    };
    assert_eq!(code, 0, "program should exit 0; stdout:\n{out}");
    assert_eq!(count_lines(&out, "1"), 1, "b (holding 1) dropped once; stdout:\n{out}");
    assert_eq!(count_lines(&out, "2"), 2, "println(2) + drop a(2); stdout:\n{out}");
}

#[test]
fn discriminator_early_return_no_double_drop() {
    let Some((code, out)) = run_stdout(
        r#"
fn consume(x: Resource) {
}
fn f(c: bool) {
    let a = Resource{id: 1}
    if c {
        consume(a)
        return
    }
    println(a.id)
}
fn main() -> i64 {
    f(true)
    return 0
}
"#,
    ) else {
        return;
    };
    assert_eq!(code, 0, "program should exit 0; stdout:\n{out}");
    assert_eq!(
        count_lines(&out, "1"),
        1,
        "early-return-after-move must drop `a` exactly once (not double); stdout:\n{out}"
    );
}

#[test]
fn discriminator_reassignment_no_leak() {
// installs id 2; scope end drops it (prints 2). both values are dropped, none leaked.
    let Some((code, out)) = run_stdout(
        r#"
fn main() -> i64 {
    let mut a = Resource{id: 1}
    a = Resource{id: 2}
    println(a.id)
    return 0
}
"#,
    ) else {
        return;
    };
    assert_eq!(code, 0, "program should exit 0; stdout:\n{out}");
    assert_eq!(
        count_lines(&out, "1"),
        1,
        "the old value (1) must be dropped on reassignment, not leaked; stdout:\n{out}"
    );
    assert_eq!(
        count_lines(&out, "2"),
        2,
        "println(2) + scope-end drop of the new value (2); stdout:\n{out}"
    );
}

#[test]
fn affine_drop_in_if_expr_branch() {
    let Some((code, out)) = run_stdout(
        r#"
fn main() -> i64 {
    let c = true
    let x = if c {
        let r = Resource{id: 43}
        println(100 + r.id)
        1
    } else {
        2
    }
    return x
}
"#,
    ) else {
        return;
    };
// x is the then-branch tail, so the drop must not clobber the result: main still returns 1
    assert_eq!(code, 1, "the branch tail value (1) must survive the drop; stdout:\n{out}");
    assert_eq!(
        count_lines(&out, "43"),
        1,
        "the branch-local `r` must be dropped exactly once (no leak, no double-drop); stdout:\n{out}"
    );
}

#[test]
fn affine_drop_order_is_reverse() {
    let Some((code, out)) = run_stdout(
        r#"
fn main() -> i64 {
    {
        let a = Resource{id: 1}
        let b = Resource{id: 2}
        let c = Resource{id: 3}
    }
    return 0
}
"#,
    ) else {
        return;
    };
    assert_eq!(code, 0, "program should exit 0; stdout:\n{out}");
    let drops: Vec<&str> = out
        .lines()
        .map(|l| l.trim())
        .filter(|l| *l == "1" || *l == "2" || *l == "3")
        .collect();
    assert_eq!(
        drops,
        vec!["3", "2", "1"],
        "affine drops must fire in reverse-declaration (lifo) order; stdout:\n{out}"
    );
}


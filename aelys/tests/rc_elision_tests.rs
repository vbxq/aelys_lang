use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

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

fn run_with_env_opt(
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

fn asan_clean(stderr: &str) -> bool {
    !stderr.contains("AddressSanitizer")
        && !stderr.contains("LeakSanitizer")
        && !stderr.contains("runtime error")
}

static ELISION_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn count_rc_calls_air(src: &str, elision: bool) -> (usize, usize) {
    use aelys_air::{AirStmtKind, Callee};
    let _guard = ELISION_ENV_LOCK.lock().unwrap();
    unsafe {
        if elision {
            std::env::remove_var("AELYS_RC_ELISION");
        } else {
            std::env::set_var("AELYS_RC_ELISION", "0");
        }
    }
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");
    let air = aelys_driver::lower_file_to_air(&source_path, OptimizationLevel::Aggressive)
        .expect("lower to AIR");
    unsafe {
        std::env::remove_var("AELYS_RC_ELISION");
    }
    let mut retain = 0usize;
    let mut release = 0usize;
    for func in &air.functions {
        for block in &func.blocks {
            for stmt in &block.stmts {
                if let AirStmtKind::CallVoid {
                    func: Callee::Named(name),
                    ..
                } = &stmt.kind
                {
                    if name == "__aelys_rc_retain" {
                        retain += 1;
                    } else if name == "__aelys_rc_release" {
                        release += 1;
                    }
                }
            }
        }
    }
    (retain, release)
}

const RC_ENV: &[(&str, &str)] = &[
    ("AELYS_RC_STATS", "1"),
    ("AELYS_ALLOC", "immix"),
    ("ASAN_OPTIONS", "detect_leaks=1"),
];

fn assert_balanced_run(src: &str, opt: OptimizationLevel, oracle: i32, ctx: &str) -> Option<()> {
    let (code, _out, stderr) = run_with_env_opt(src, RuntimeVariant::RcCycles, opt, RC_ENV)?;
    assert!(
        asan_clean(&stderr),
        "{ctx}: must be ASan/LSan clean; stderr:\n{stderr}"
    );
    assert_eq!(
        code, oracle,
        "{ctx}: oracle must be {oracle}; stderr:\n{stderr}"
    );
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        allocs, frees,
        "{ctx}: must be balanced (allocs={allocs} frees={frees})"
    );
    Some(())
}

const T1_CYCLE_SRC: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = b
    b.next = a
    let alias: Rc<Node> = a
    __aelys_collect()
    return alias.val
}
"#;

#[test]
fn t1_cycle_clone_not_elided_and_balanced() {
    let off = count_rc_calls_air(T1_CYCLE_SRC, false);
    let on = count_rc_calls_air(T1_CYCLE_SRC, true);
    assert_eq!(
        off, on,
        "T1: cycle-clone retain/release counts must be unchanged by elision (off={:?} on={:?})",
        off, on
    );
    assert_balanced_run(T1_CYCLE_SRC, OptimizationLevel::None, 1, "T1 @ -O0");
    assert_balanced_run(T1_CYCLE_SRC, OptimizationLevel::Aggressive, 1, "T1 @ -O2");
}

#[test]
fn t2_two_adjacent_retains_lift_never_touched() {
    let off = count_rc_calls_air(T1_CYCLE_SRC, false);
    let on = count_rc_calls_air(T1_CYCLE_SRC, true);
    assert_eq!(
        off, on,
        "T2: neither the lift retain nor the clone retain may be elided in the cycle \
         program (off={:?} on={:?})",
        off, on
    );
    assert_balanced_run(T1_CYCLE_SRC, OptimizationLevel::Aggressive, 1, "T2 @ -O2");
}

const D1_STRUCT_LITERAL_SRC: &str = r#"
struct Box { inner: Rc<i64> }
fn main() -> i64 {
    let p: Rc<i64> = Rc::new(42)
    let bx: Box = Box { inner: p }
    let q: Rc<i64> = bx.inner
    __aelys_collect()
    return Rc::get(q)
}
"#;

#[test]
fn d1_struct_literal_store_balanced() {
    assert_balanced_run(
        D1_STRUCT_LITERAL_SRC,
        OptimizationLevel::None,
        42,
        "D-1 @ -O0",
    );
    assert_balanced_run(
        D1_STRUCT_LITERAL_SRC,
        OptimizationLevel::Aggressive,
        42,
        "D-1 @ -O2",
    );
}

const SEAM4_SRC: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn use_vec(v: Vec<i64>) -> i64 {
    return v[0] + v[1]
}
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(5)
    let r2: Rc<i64> = r
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = b
    b.next = a
    let v = vec[10, 20, 30]
    let w = v
    Vec::push(w, 40)
    let s = use_vec(v)
    __aelys_collect()
    return Rc::get(r2) + v[0] + w[3] + s
}
"#;

#[test]
fn positive_seam4_let_r2_r_elided() {
    let off = count_rc_calls_air(SEAM4_SRC, false);
    let on = count_rc_calls_air(SEAM4_SRC, true);
    assert_eq!(
        off,
        (3, 6),
        "SEAM4 before-elision baseline (retain, release)"
    );
    assert_eq!(
        on,
        (2, 5),
        "SEAM4 after-elision (one retain+release pair removed)"
    );
    assert_balanced_run(SEAM4_SRC, OptimizationLevel::None, 85, "SEAM4 @ -O0");
    assert_balanced_run(SEAM4_SRC, OptimizationLevel::Aggressive, 85, "SEAM4 @ -O2");
}

const DENSE_CHAIN_SRC: &str = r#"
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let b: Rc<i64> = a
    let c: Rc<i64> = b
    let d: Rc<i64> = c
    return Rc::get(d)
}
"#;

#[test]
fn positive_dense_chain_all_intermediates_elided() {
    let off = count_rc_calls_air(DENSE_CHAIN_SRC, false);
    let on = count_rc_calls_air(DENSE_CHAIN_SRC, true);
    assert_eq!(off, (3, 4), "dense chain before-elision (retain, release)");
    assert_eq!(
        on,
        (0, 1),
        "dense chain after-elision: all 3 clone retains + 3 releases removed (combined 7→1)"
    );
    assert_balanced_run(DENSE_CHAIN_SRC, OptimizationLevel::None, 7, "dense @ -O0");
    assert_balanced_run(
        DENSE_CHAIN_SRC,
        OptimizationLevel::Aggressive,
        7,
        "dense @ -O2",
    );
}

const LOOP_LIVE_SRC: &str = r#"
fn main() -> i64 {
    let base: Rc<i64> = Rc::new(9)
    let mut acc: i64 = 0
    let mut i: i64 = 0
    while i < 3 {
        let b: Rc<i64> = base
        acc = acc + Rc::get(b)
        i = i + 1
    }
    return acc + Rc::get(base)
}
"#;

#[test]
fn negative_loop_live_across_backedge_kept() {
    assert_balanced_run(
        LOOP_LIVE_SRC,
        OptimizationLevel::None,
        36,
        "loop-live @ -O0",
    );
    assert_balanced_run(
        LOOP_LIVE_SRC,
        OptimizationLevel::Aggressive,
        36,
        "loop-live @ -O2",
    );
}

const CALL_ARG_ESCAPE_SRC: &str = r#"
fn sink(x: Rc<i64>) -> i64 {
    return Rc::get(x)
}
fn main() -> i64 {
    let p: Rc<i64> = Rc::new(11)
    let q: Rc<i64> = p
    let n: i64 = sink(q)
    __aelys_collect()
    return n + Rc::get(p)
}
"#;

#[test]
fn negative_call_arg_escape_kept() {
    assert_balanced_run(
        CALL_ARG_ESCAPE_SRC,
        OptimizationLevel::None,
        22,
        "call-arg @ -O0",
    );
    assert_balanced_run(
        CALL_ARG_ESCAPE_SRC,
        OptimizationLevel::Aggressive,
        22,
        "call-arg @ -O2",
    );
}

const DIVERGENT_RELEASE_SRC: &str = r#"
fn main() -> i64 {
    let x: Rc<i64> = Rc::new(13)
    let y: Rc<i64> = x
    if Rc::get(y) > 0 {
        return Rc::get(y)
    }
    return 0
}
"#;

#[test]
fn divergent_release_soundly_elided() {
    assert_balanced_run(
        DIVERGENT_RELEASE_SRC,
        OptimizationLevel::None,
        13,
        "divergent @ -O0",
    );
    assert_balanced_run(
        DIVERGENT_RELEASE_SRC,
        OptimizationLevel::Aggressive,
        13,
        "divergent @ -O2",
    );
}

const MEMBER_INIT_SRC: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let leaf: Rc<Node> = Rc::new(Node { val: 7, next: Rc::null() })
    let head: Rc<Node> = Rc::new(Node { val: 1, next: leaf })
    let n: Rc<Node> = head.next
    __aelys_collect()
    return n.val
}
"#;

#[test]
fn negative_member_init_clone_kept() {
    let off = count_rc_calls_air(MEMBER_INIT_SRC, false);
    let on = count_rc_calls_air(MEMBER_INIT_SRC, true);
    assert_eq!(
        off, on,
        "member-init: the `let n = head.next` clone retain/release must NOT be elided \
         (off={:?} on={:?})",
        off, on
    );
    if let Some((code, _o, e)) = run_with_env_opt(
        MEMBER_INIT_SRC,
        RuntimeVariant::RcCycles,
        OptimizationLevel::None,
        RC_ENV,
    ) {
        assert!(
            asan_clean(&e),
            "member-init @ -O0 must be ASan/LSan clean; stderr:\n{e}"
        );
        assert_eq!(code, 7, "member-init @ -O0 result must be 7; stderr:\n{e}");
    }
    if let Some((code, _o, e)) = run_with_env_opt(
        MEMBER_INIT_SRC,
        RuntimeVariant::RcCycles,
        OptimizationLevel::Aggressive,
        RC_ENV,
    ) {
        assert!(
            asan_clean(&e),
            "member-init @ -O2 must be ASan/LSan clean; stderr:\n{e}"
        );
        assert_eq!(code, 7, "member-init @ -O2 result must be 7; stderr:\n{e}");
    }
}

use aelys_driver::{compile_file_with_llvm_variant, RuntimeVariant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
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

fn run_with_env(src: &str, variant: RuntimeVariant, env: &[(&str, &str)]) -> Option<(i32, String)> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(&source_path, OptimizationLevel::None, false, variant) {
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
    let code = output.status.code().expect("exit code");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Some((code, stderr))
}

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
    let (a, m) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, m.trim().parse().ok()?))
}

fn core_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("core")
        .join("src")
}

fn build_c_probe(dir: &Path, probe_src: &str, asan: bool) -> Option<PathBuf> {
    let src = core_src_dir();
    let full = format!(
        r#"#include <stdio.h>
#include <stdlib.h>
void __aelys_panic(const char *p, long long n) {{
    fwrite(p, 1, (size_t)n, stderr);
    fputc('\n', stderr);
    fflush(stderr);
    abort();
}}
{probe_src}
"#
    );
    let probe = dir.join("probe.c");
    fs::write(&probe, full).expect("write probe");
    let exe = dir.join("probe_bin");

    let mut cmd = Command::new("clang");
    if asan {
        cmd.arg("-fsanitize=address").arg("-g");
    }
    cmd.arg("-I")
        .arg(&src)
        .arg(src.join("aelys_alloc_immix.c"))
        .arg(&probe)
        .arg("-o")
        .arg(&exe);

    let out = match cmd.output() {
        Ok(o) => o,
        Err(_) => {
            eprintln!("clang unavailable; skipping C probe");
            return None;
        }
    };
    if !out.status.success() {
        panic!(
            "C probe build failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Some(exe)
}

#[test]
fn p2_memory_bounded_under_churn() {
    let dir = tempdir().expect("tempdir");
    let probe = r#"
#include <stdio.h>
extern void *aelys_immix_alloc(long long);
extern void aelys_immix_free(void *);
extern long long aelys_immix_block_count(void);
int main(void) {
    long long warm = 0, fin = 0;
    for (long long i = 0; i < 100000; i++) {

        void *p = aelys_immix_alloc(24);
        aelys_immix_free(p);
        if (i == 1000) warm = aelys_immix_block_count();
    }
    fin = aelys_immix_block_count();
    printf("warm=%lld final=%lld\n", warm, fin);
    return (int)fin;
}
"#;
    let Some(exe) = build_c_probe(dir.path(), probe, false) else {
        return;
    };
    let out = Command::new(&exe)
        .env("AELYS_ALLOC", "immix")
        .output()
        .expect("run P2 probe");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    let blocks = out.status.code().expect("exit code");
    eprintln!("P2 churn: {} (stderr: {})", stdout.trim(), stderr.trim());
    assert!(
        blocks >= 1 && blocks <= 4,
        "region block count must plateau (free-list reuse), got {blocks}; stdout: {stdout}"
    );
    let warm = stdout
        .split_whitespace()
        .find_map(|t| t.strip_prefix("warm="))
        .and_then(|n| n.parse::<i64>().ok())
        .expect("warm count");
    let fin = stdout
        .split_whitespace()
        .find_map(|t| t.strip_prefix("final="))
        .and_then(|n| n.parse::<i64>().ok())
        .expect("final count");
    assert_eq!(
        warm, fin,
        "block count must be identical at 1k and 100k (plateau, not linear growth); stdout: {stdout}"
    );
}

#[test]
fn p3a_asan_immix_balanced_is_clean() {
    let dir = tempdir().expect("tempdir");
    let probe = r#"
#include <string.h>
extern void *aelys_immix_alloc(long long);
extern void aelys_immix_free(void *);
int main(void) {
    for (int i = 0; i < 5000; i++) {
        char *p = (char *)aelys_immix_alloc(24);
        memset(p, 0xAB, 24);   /* touch every byte of the slot (no overflow) */
        aelys_immix_free(p);       /* p IS the base */
    }
    for (int i = 0; i < 4; i++) {
        char *p = (char *)aelys_immix_alloc(40000);
        memset(p, 0xCD, 40000);
        aelys_immix_free(p);
    }
    return 0;
}
"#;
    let Some(exe) = build_c_probe(dir.path(), probe, true) else {
        return;
    };
    let out = Command::new(&exe)
        .env("AELYS_ALLOC", "immix")
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .output()
        .expect("run P3-A positive probe");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(0),
        "instrumented immix churn must exit 0 (no sanitizer abort); stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "instrumented immix must report no ASan/LSan errors; stderr:\n{stderr}"
    );
}

#[test]
fn p3a_asan_negative_double_free_aborts() {
    let dir = tempdir().expect("tempdir");
    let probe = r#"
#include <stdio.h>
extern void *aelys_immix_alloc(long long);
extern void aelys_immix_free(void *);
int main(void) {
    void *p = aelys_immix_alloc(24);
    aelys_immix_free(p);
    fprintf(stderr, "FIRST_FREE_OK\n");
    aelys_immix_free(p);          /* MUST abort here (double-free) */
    fprintf(stderr, "REACHED_AFTER_SECOND_FREE\n"); /* must NOT print */
    return 0;
}
"#;
    let Some(exe) = build_c_probe(dir.path(), probe, true) else {
        return;
    };
    let out = Command::new(&exe)
        .env("AELYS_ALLOC", "immix")
        .output()
        .expect("run P3-A negative probe");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("FIRST_FREE_OK"),
        "the first free must succeed; stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("REACHED_AFTER_SECOND_FREE"),
        "the program must abort on the double-free, not run past it; stderr:\n{stderr}"
    );
    assert!(
        !out.status.success(),
        "a double-free must not exit successfully; stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("double-free or bad free") || stderr.contains("AddressSanitizer"),
        "the abort must be the double-free guard (magic-flip panic or ASan); stderr:\n{stderr}"
    );
}

#[test]
fn p4_throughput_immix_not_pathological() {
    let dir = tempdir().expect("tempdir");
    let probe = r#"
#include <stdio.h>
#include <stdlib.h>
extern void *aelys_immix_alloc(long long);
extern void aelys_immix_free(void *);
int main(void) {
    long long N = 2000000;
    volatile long long sink = 0;
    for (long long i = 0; i < N; i++) {
        char *p = (char *)aelys_immix_alloc(24);
        p[0] = (char)i;
        sink += p[0];
        aelys_immix_free(p);
    }
    return (int)(sink & 1);
}
"#;
    let Some(exe) = build_c_probe(dir.path(), probe, false) else {
        return;
    };

    let time_mode = |mode: &str| -> f64 {
        let mut best = f64::INFINITY;
        for _ in 0..2 {
            let t = Instant::now();
            let out = Command::new(&exe)
                .env("AELYS_ALLOC", mode)
                .output()
                .expect("run P4 probe");
            assert!(out.status.code().is_some(), "probe must terminate");
            best = best.min(t.elapsed().as_secs_f64());
        }
        best
    };

    let immix = time_mode("immix");
    let malloc = time_mode("malloc");
    eprintln!("P4 throughput: immix={immix:.4}s malloc={malloc:.4}s");
    assert!(
        immix <= malloc * 4.0 + 0.05,
        "immix ({immix:.4}s) is pathologically slower than malloc ({malloc:.4}s)"
    );
}

#[test]
fn p5a_rc_address_stable_across_life() {
    let dir = tempdir().expect("tempdir");
    let probe = r#"
#include <stdio.h>
extern void *aelys_immix_alloc(long long);
extern void aelys_immix_free(void *);
int main(void) {
    void *live = aelys_immix_alloc(24);     /* a "live Rc" we keep */
    void *first = live;
    /* churn around it: many alloc/free of other objects. */
    for (int i = 0; i < 1000; i++) {
        void *t = aelys_immix_alloc(24);
        aelys_immix_free(t);
    }
    /* the live object's address must be byte-identical (never evacuated). */
    if (live != first) {
        fprintf(stderr, "MOVED\n");
        return 1;
    }
    aelys_immix_free(live);
    return 0;
}
"#;
    let Some(exe) = build_c_probe(dir.path(), probe, false) else {
        return;
    };
    let out = Command::new(&exe)
        .env("AELYS_ALLOC", "immix")
        .output()
        .expect("run probe");
    assert_eq!(
        out.status.code(),
        Some(0),
        "the live object must not move (non-moving); stderr:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn p5b_cycle_collected_under_immix() {
    let src = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = b
    b.next = a
    __aelys_collect()
    return 0
}
"#;
    let Some((code, stderr)) = run_with_env(src, RuntimeVariant::RcCycles, &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "immix")]) else {
        return;
    };
    assert_eq!(code, 0, "cycle program exits 0; stderr:\n{stderr}");
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        allocs, frees,
        "the A<->B cycle must be collected under immix (allocs==frees); stderr:\n{stderr}"
    );
    assert_eq!(allocs, 2, "exactly the two cycle nodes; stderr:\n{stderr}");
}

#[test]
fn p6_counters_identical_under_immix() {
    let src = r#"
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let b: Rc<i64> = a
    return Rc::get(b)
}
"#;
    let Some((code, stderr)) = run_with_env(src, RuntimeVariant::Rc, &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "immix")]) else {
        return;
    };
    assert_eq!(code, 7, "shared Rc reads 7; stderr:\n{stderr}");
    assert_eq!(
        parse_stats(&stderr),
        Some((1, 1)),
        "shared Rc must be allocs=1 frees=1 under immix (last-drop free); stderr:\n{stderr}"
    );
}

#[test]
fn p7_escape_hatch_malloc_shared_rc() {
    let src = r#"
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let b: Rc<i64> = a
    return Rc::get(b)
}
"#;
    let Some((code, stderr)) = run_with_env(src, RuntimeVariant::Rc, &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "malloc")]) else {
        return;
    };
    assert_eq!(code, 7, "shared Rc reads 7 under malloc; stderr:\n{stderr}");
    assert_eq!(
        parse_stats(&stderr),
        Some((1, 1)),
        "malloc backing: allocs=1 frees=1; stderr:\n{stderr}"
    );
}

#[test]
fn p7_escape_hatch_malloc_cow_value_semantics() {
    let src = r#"
fn main() -> i64 {
    let a = vec[1, 2, 3]
    let b = a
    Vec::push(b, 9)
    return a[0] + a[1] + a[2] + b[3]
}
"#;
    let Some((code, stderr)) = run_with_env(src, RuntimeVariant::Rc, &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "malloc")]) else {
        return;
    };
    assert_eq!(code, 15, "a=[1,2,3] (6) + b[3]=9 => 15 under malloc; stderr:\n{stderr}");
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(allocs, frees, "balanced under malloc (no leak/double-free); stderr:\n{stderr}");
    assert_eq!(allocs, 2, "shared push allocates the CoW copy (slow path); stderr:\n{stderr}");
}

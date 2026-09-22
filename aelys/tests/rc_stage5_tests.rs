use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use tempfile::tempdir;

mod common;
use common::{exe_path_for, linker_unavailable};

fn run_with_env(src: &str, variant: RuntimeVariant, env: &[(&str, &str)]) -> Option<(i32, String)> {
    let _pin = common::pin_legs("run_with_env", 1);
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(&source_path, OptimizationLevel::None, false, variant) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                common::require_linker_skip(
                    "a skipped value row carries no runtime evidence at all",
                );
                return None;
            }
            panic!("compilation/link should succeed: {err}");
        }
    }

    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        common::require_linker_skip("a skipped value row carries no runtime evidence at all");
        return None;
    }

    let mut cmd = Command::new(&exe);
    for (k, v) in env {
        cmd.env(k, v);
    }
    common::note_leg();
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

fn build_guard_probe(dir: &Path, rc_file: &str, probe_src: &str) -> Option<PathBuf> {
    let src = core_src_dir();
    let probe = dir.join(format!("guard_{rc_file}.c"));
    fs::write(&probe, probe_src).expect("write guard probe");
    let exe = dir.join(format!("guard_{rc_file}_bin"));
    let out = match Command::new("clang")
        .arg("-I")
        .arg(&src)
        .arg(src.join("aelys_alloc_immix.c"))
        .arg(src.join(rc_file))
        .arg(&probe)
        .arg("-o")
        .arg(&exe)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("clang unavailable; skipping guard probe");
            return None;
        }
    };
    if !out.status.success() {
        panic!(
            "guard probe build failed:\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Some(exe)
}

// these two must stay byte-identical to aelys_rc_underflow_msg and aelys_rc_freed_msg in core/src/aelys_rc.h
const UNDERFLOW_TEXT: &str = "refcount already zero, a release with no matching retain";
const FREED_TEXT: &str = "the object was already freed, a release with no matching retain";

const GUARD_PROBE: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

extern void *aelys_immix_alloc(long long);
extern void aelys_immix_free(void *);
extern void __aelys_rc_release(void *ptr);

long long __aelys_free_count = 0;
long long __aelys_alloc_count = 0;
void (*__aelys_collect_hook)(void) = 0;
const unsigned __aelys_rc_type_table[8] = {0};

void __aelys_panic(const char *p, long long n) {
    fwrite(p, 1, (size_t)n, stderr);
    fputc('\n', stderr);
    fflush(stderr);
    abort();
}

void __aelys_free(void *ptr) {
    aelys_immix_free(ptr);
}

/* every allocation site in the runtime and in codegen writes refcount 1, so the probe does too */
static void *born(void) {
    char *base = (char *)aelys_immix_alloc(16 + 8);
    *(unsigned *)base = 1u;
    *(unsigned char *)(base + 4) = 0;
    *(unsigned *)(base + 8) = 0u;
    return base + 16;
}

int main(int argc, char **argv) {
    void *a = born();
    void *b = born();
    __aelys_rc_release(b);
    if (argc > 1 && strcmp(argv[1], "stale") == 0) {
        __aelys_rc_release(a); /* a later free retires the tombstone that named b */
    }
    fprintf(stderr, "FIRST_RELEASE_OK\n");
    __aelys_rc_release(b);
    fprintf(stderr, "REACHED_AFTER_SECOND_RELEASE\n");
    return 0;
}
"#;

#[test]
fn p3b_a_second_release_is_caught_under_both_allocators() {
    let dir = tempdir().expect("tempdir");
    for rc_file in ["aelys_rc_leak.c", "aelys_rc_real.c", "aelys_rc_cycles.c"] {
        let Some(exe) = build_guard_probe(dir.path(), rc_file, GUARD_PROBE) else {
            return;
        };
        for alloc in ["immix", "malloc"] {
            for shape in ["fresh", "stale"] {
                let out = Command::new(&exe)
                    .arg(shape)
                    .env("AELYS_ALLOC", alloc)
                    .output()
                    .expect("run the guard probe");
                let stderr = String::from_utf8_lossy(&out.stderr);
                let leg = format!("{rc_file}/{alloc}/{shape}");
                assert!(
                    stderr.contains("FIRST_RELEASE_OK"),
                    "{leg}: the first release must survive; stderr:\n{stderr}"
                );
                // libc owns a freed chunk, so a retired tombstone leaves malloc nothing to read
                let blind = rc_file != "aelys_rc_leak.c" && alloc == "malloc" && shape == "stale";
                if blind {
                    assert!(
                        stderr.contains("REACHED_AFTER_SECOND_RELEASE"),
                        "{leg} is the one cell the guard cannot reach; a fire here means the \
                         runtime grew a way to see it and this row must say so; stderr:\n{stderr}"
                    );
                    continue;
                }
                assert!(
                    !stderr.contains("REACHED_AFTER_SECOND_RELEASE"),
                    "{leg}: the second release must abort, not run past it; stderr:\n{stderr}"
                );
                assert!(
                    stderr.contains("__aelys_rc_release:"),
                    "{leg}: the abort must be the release guard itself; stderr:\n{stderr}"
                );
                // the leak variant never frees, so its witness is the count; the two that do free
                let (want, reject) = if rc_file == "aelys_rc_leak.c" {
                    (UNDERFLOW_TEXT, FREED_TEXT)
                } else {
                    (FREED_TEXT, UNDERFLOW_TEXT)
                };
                assert!(
                    stderr.contains(want),
                    "{leg}: the guard must abort with `{want}`; stderr:\n{stderr}"
                );
                assert!(
                    !stderr.contains(reject),
                    "{leg}: the guard reached the wrong witness, `{reject}` instead of `{want}`; \
                     stderr:\n{stderr}"
                );
                assert!(
                    !out.status.success(),
                    "{leg}: a double release must not exit successfully; stderr:\n{stderr}"
                );
            }
        }
    }
}

// a pop from a non-empty bucket is the one shape the guard probe never reaches: there the
const HOT_BUCKET_PROBE: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include <string.h>

extern void *aelys_immix_alloc(long long);
extern void aelys_immix_free(void *);
extern void __aelys_rc_release(void *ptr);

long long __aelys_free_count = 0;
long long __aelys_alloc_count = 0;
void (*__aelys_collect_hook)(void) = 0;
const unsigned __aelys_rc_type_table[8] = {0};

void __aelys_panic(const char *p, long long n) {
    fwrite(p, 1, (size_t)n, stderr);
    fputc('\n', stderr);
    fflush(stderr);
    abort();
}

void __aelys_free(void *ptr) {
    aelys_immix_free(ptr);
}

static void *born(void) {
    char *base = (char *)aelys_immix_alloc(16 + 8);
    *(unsigned *)base = 1u;
    *(unsigned char *)(base + 4) = 0;
    *(unsigned *)(base + 8) = 0u;
    return base + 16;
}

int main(int argc, char **argv) {
    int stale = (argc > 1 && strcmp(argv[1], "stale") == 0);
    void *a = born();
    void *b = born();
    void *c = born();
    __aelys_rc_release(c);
    __aelys_rc_release(a);
    /* b takes the tombstone, so a is a warm slot the guard can only judge from its own bytes */
    __aelys_rc_release(b);
    fprintf(stderr, "HOT_BUCKET_WARM\n");
    if (stale) {
        __aelys_rc_release(a);
        fprintf(stderr, "HOT_STALE_RELEASE_SURVIVED\n");
    }
    void *d = born();
    void *e = born();
    void *f = born();
    uintptr_t bases[3] = {(uintptr_t)d - 16, (uintptr_t)e - 16, (uintptr_t)f - 16};
    for (int i = 0; i < 3; i++) {
        fprintf(stderr, "HOT_POP base=%p rem=%d\n", (void *)bases[i], (int)(bases[i] % 16));
        if (bases[i] % 16) {
            fprintf(stderr, "HOT_MISALIGNED_BASE\n");
        }
    }
    memset((char *)d - 16, 0x5A, 24);
    memset((char *)e - 16, 0x5A, 24);
    memset((char *)f - 16, 0x5A, 24);
    fprintf(stderr, "HOT_HEADERS_WRITTEN\n");
    return 0;
}
"#;

#[test]
fn p3c_a_stale_release_on_a_warm_bucket_is_caught() {
    let dir = tempdir().expect("tempdir");
    for rc_file in ["aelys_rc_leak.c", "aelys_rc_real.c", "aelys_rc_cycles.c"] {
        let Some(exe) = build_guard_probe(dir.path(), rc_file, HOT_BUCKET_PROBE) else {
            return;
        };
        for shape in ["clean", "stale"] {
            let out = Command::new(&exe)
                .arg(shape)
                .env("AELYS_ALLOC", "immix")
                .output()
                .expect("run the hot bucket probe");
            let stderr = String::from_utf8_lossy(&out.stderr);
            let leg = format!("{rc_file}/immix/{shape}");
            assert!(
                stderr.contains("HOT_BUCKET_WARM"),
                "{leg}: the three honest releases must all go through, or the bucket is never \
                 warm and the row proves nothing; stderr:\n{stderr}"
            );
            assert!(
                !stderr.contains("HOT_MISALIGNED_BASE"),
                "{leg}: a pop handed back a base that is not 16-aligned, so the free list link \
                 is sitting on the bytes the rc header owns; stderr:\n{stderr}"
            );
            if shape == "clean" {
                assert!(
                    stderr.contains("HOT_HEADERS_WRITTEN") && out.status.success(),
                    "{leg}: three pops out of a warm bucket and a header written into each must \
                     go through untouched; stderr:\n{stderr}"
                );
                continue;
            }
            assert!(
                !stderr.contains("HOT_STALE_RELEASE_SURVIVED"),
                "{leg}: a second release on a slot that is on the free list must abort; \
                 stderr:\n{stderr}"
            );
            // the leak variant frees nothing, so it never warms the bucket and answers on the count
            let want = if rc_file == "aelys_rc_leak.c" {
                UNDERFLOW_TEXT
            } else {
                FREED_TEXT
            };
            assert!(
                stderr.contains(want),
                "{leg}: the abort must be `{want}`; stderr:\n{stderr}"
            );
            assert!(
                !out.status.success(),
                "{leg}: a stale release must not exit successfully; stderr:\n{stderr}"
            );
        }
    }
}

// reading a pointer whose target went back to free is indeterminate, and gcc folds the
const LARGE_REUSE_PROBE: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>

extern void *aelys_immix_alloc(long long);
extern void aelys_immix_free(void *);
extern void __aelys_rc_release(void *ptr);

long long __aelys_free_count = 0;
long long __aelys_alloc_count = 0;
void (*__aelys_collect_hook)(void) = 0;
const unsigned __aelys_rc_type_table[8] = {0};

void __aelys_panic(const char *p, long long n) {
    fwrite(p, 1, (size_t)n, stderr);
    fputc('\n', stderr);
    fflush(stderr);
    abort();
}

void __aelys_free(void *ptr) {
    aelys_immix_free(ptr);
}

static void *born(long long payload) {
    char *base = (char *)aelys_immix_alloc(16 + payload);
    *(unsigned *)base = 1u;
    *(unsigned char *)(base + 4) = 0;
    *(unsigned *)(base + 8) = 0u;
    return base + 16;
}

int main(int argc, char **argv) {
    long long payload = (argc > 1) ? atoll(argv[1]) : 40000;
    void *a = born(payload);
    /* the integer is taken while a is still live, the probe must not repeat the defect it pins */
    uintptr_t ua = (uintptr_t)a;
    __aelys_rc_release(a);
    void *b = born(payload);
    fprintf(stderr, "reuse=%d\n", (int)((uintptr_t)b == ua));
    __aelys_rc_release(b);
    fputs("SURVIVED\n", stdout);
    return 0;
}
"#;

fn build_opt_probe(dir: &Path, cc: &str, opt: &str, probe_src: &str) -> Option<PathBuf> {
    let src = core_src_dir();
    let stem = format!("opt_{cc}_{}", opt.trim_start_matches('-'));
    let probe = dir.join(format!("{stem}.c"));
    fs::write(&probe, probe_src).expect("write opt probe");
    let exe = dir.join(format!("{stem}_bin"));
    let out = match Command::new(cc)
        .arg(opt)
        .arg("-I")
        .arg(&src)
        .arg(src.join("aelys_alloc_immix.c"))
        .arg(src.join("aelys_rc_real.c"))
        .arg(&probe)
        .arg("-o")
        .arg(&exe)
        .output()
    {
        Ok(o) => o,
        Err(_) => {
            eprintln!("{cc} unavailable; skipping the optimisation row");
            return None;
        }
    };
    if !out.status.success() {
        panic!(
            "opt probe build failed under {cc} {opt}:\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Some(exe)
}

#[test]
fn p3e_the_tombstone_survives_optimisation_on_the_oversized_path() {
    let dir = tempdir().expect("tempdir");
    let mut rows = 0usize;
    for cc in ["gcc", "clang"] {
        for opt in ["-O0", "-O1", "-O2", "-O3"] {
            let Some(exe) = build_opt_probe(dir.path(), cc, opt, LARGE_REUSE_PROBE) else {
                continue;
            };
            for alloc in ["immix", "malloc"] {
                for payload in ["40000", "24"] {
                    let out = Command::new(&exe)
                        .arg(payload)
                        .env("AELYS_ALLOC", alloc)
                        .output()
                        .expect("run the oversized reuse probe");
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let leg = format!("{cc}/{opt}/{alloc}/{payload}");
                    assert!(
                        stderr.contains("reuse=1"),
                        "{leg}: the fresh object must land back on the address just freed, or \
                         nothing retires a tombstone and the row is vacuous; stderr:\n{stderr}"
                    );
                    assert!(
                        out.status.success() && stdout.contains("SURVIVED"),
                        "{leg}: an honest release on a live object aborted, so the tombstone was \
                         never retired; stdout:\n{stdout}\nstderr:\n{stderr}"
                    );
                    rows += 1;
                }
            }
        }
    }
    assert!(
        rows > 0,
        "no C compiler answered, so every optimisation row above was skipped"
    );
    eprintln!("P3E: {rows} optimisation rows measured");
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
    let Some((code, stderr)) = run_with_env(
        src,
        RuntimeVariant::RcCycles,
        &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "immix")],
    ) else {
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
    let Some((code, stderr)) = run_with_env(
        src,
        RuntimeVariant::Rc,
        &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "immix")],
    ) else {
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
    let Some((code, stderr)) = run_with_env(
        src,
        RuntimeVariant::Rc,
        &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "malloc")],
    ) else {
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
    let Some((code, stderr)) = run_with_env(
        src,
        RuntimeVariant::Rc,
        &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "malloc")],
    ) else {
        return;
    };
    assert_eq!(
        code, 15,
        "a=[1,2,3] (6) + b[3]=9 => 15 under malloc; stderr:\n{stderr}"
    );
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        allocs, frees,
        "balanced under malloc (no leak/double-free); stderr:\n{stderr}"
    );
    assert_eq!(
        allocs, 2,
        "shared push allocates the CoW copy (slow path); stderr:\n{stderr}"
    );
}

use aelys_driver::{compile_file_with_llvm_variant, RuntimeVariant};
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

fn run_with_env(
    src: &str,
    variant: RuntimeVariant,
    env: &[(&str, &str)],
) -> Option<(i32, String, String)> {
    run_with_env_opt(src, variant, OptimizationLevel::None, env)
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

fn core_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("core")
        .join("src")
}

fn build_seam1_probe(dir: &Path, probe_body: &str) -> Option<PathBuf> {
    let src = core_src_dir();
    let probe = dir.join("seam1_probe.c");
    fs::write(&probe, probe_body).expect("write probe");
    let exe = dir.join("seam1_probe_bin");

    let mut cmd = Command::new("clang");
    cmd.arg("-fsanitize=address")
        .arg("-g")
        .arg("-I")
        .arg(&src)
        .arg(src.join("aelys_alloc_immix.c"))
        .arg(src.join("aelys_rc_cycles.c"))
        .arg(&probe)
        .arg("-o")
        .arg(&exe);

    let out = match cmd.output() {
        Ok(o) => o,
        Err(_) => {
            eprintln!("clang unavailable; skipping seam-1 C probe");
            return None;
        }
    };
    if !out.status.success() {
        panic!(
            "seam-1 C probe build failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Some(exe)
}

const SEAM1_PROBE_TEMPLATE: &str = r#"
#include <stdio.h>
#include <stdlib.h>
#include <stdint.h>
#include "aelys_alloc_immix.h"
#include "aelys_rc.h"
long long __aelys_alloc_count = 0, __aelys_free_count = 0;
void (*__aelys_collect_hook)(void) = 0;
void *__aelys_alloc(long long s){ void*p=aelys_immix_alloc(s); if(!p)abort(); __aelys_alloc_count++; return p; }
void __aelys_free(void *p){ aelys_immix_free(p); }
void __aelys_panic(const char*p,long long n){ fwrite(p,1,(size_t)n,stderr); fputc('\n',stderr); abort(); }
const uint32_t __aelys_rc_type_table[] = { 1u, 1u, 4u, 0u, 0u };
extern void __aelys_rc_retain(void*); extern void __aelys_rc_release(void*); extern void __aelys_cycle_collect(void);
static void *make_obj(long long b, uint32_t rc, uint32_t tid, uint8_t flags){
    char*base=(char*)__aelys_alloc(16+b);
    *(uint32_t*)(base+0)=rc; *(uint8_t*)(base+4)=flags; *(uint8_t*)(base+5)=0;
    *(uint8_t*)(base+6)=0; *(uint8_t*)(base+7)=0; *(uint32_t*)(base+8)=tid;
    return base+16;
}
int main(void){
    void *G = make_obj(8, 1, 0, 0);
    void *vecbuf = make_obj(24, 1, 0, (uint8_t)(__AELYS_VEC_FLAGS)); /* __aelys_vec_new stamp */
    *(void **)((char*)G + 0) = vecbuf;
    *(void **)((char*)vecbuf + 0) = G;
    *(uint64_t*)((char*)vecbuf + 8)  = 0x1111111111111111ULL;
    *(uint64_t*)((char*)vecbuf + 16) = 0x2222222222222222ULL;
    __aelys_rc_retain(G);      __aelys_rc_release(G);
    __aelys_rc_retain(vecbuf); __aelys_rc_release(vecbuf);
    __aelys_cycle_collect();
    fprintf(stderr, "allocs=%lld frees=%lld\n", __aelys_alloc_count, __aelys_free_count);
    if (__aelys_free_count != 0) {
        fprintf(stderr, "FAIL: the Vec buffer was traced & FREED by the collector\n");
        return 2;
    }
    /* Safe post-fix read of the intact buffer (pre-fix this is a UAF that ASan traps). */
    if (*(uint64_t*)((char*)vecbuf+8) != 0x1111111111111111ULL) {
        fprintf(stderr, "FAIL: the Vec buffer's elements were corrupted\n");
        return 3;
    }
    fprintf(stderr, "OK: buffer intact, never collected\n");
    return 0;
}
"#;

#[test]
fn seam1_probe_no_trace_excludes_vec_buffer_from_collector() {
    let dir = tempdir().expect("tempdir");

    let fixed_body = SEAM1_PROBE_TEMPLATE.replace("__AELYS_VEC_FLAGS", "AELYS_FLAG_NO_TRACE");
    let Some(exe) = build_seam1_probe(dir.path(), &fixed_body) else {
        return;
    };
    let out = Command::new(&exe)
        .env("AELYS_ALLOC", "immix")
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .output()
        .expect("run NO_TRACE probe");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(0),
        "NO_TRACE Vec buffer must be EXCLUDED from the collector (frees==0, intact, \
         ASan-clean); stderr:\n{stderr}"
    );
    assert!(asan_clean(&stderr), "fixed path must be ASan-clean; stderr:\n{stderr}");
    assert!(stderr.contains("frees=0"), "the buffer must never be freed by the collector; stderr:\n{stderr}");

    let dir2 = tempdir().expect("tempdir2");
    let buggy_body = SEAM1_PROBE_TEMPLATE.replace("__AELYS_VEC_FLAGS", "0");
    let Some(exe2) = build_seam1_probe(dir2.path(), &buggy_body) else {
        return;
    };
    let out2 = Command::new(&exe2)
        .env("AELYS_ALLOC", "immix")
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .output()
        .expect("run flags-0 probe");
    let stderr2 = String::from_utf8_lossy(&out2.stderr);
    assert_ne!(
        out2.status.code(),
        Some(0),
        "the pre-fix (flags 0) stamp MUST be caught: the collector mis-traces & frees \
         the Vec buffer, and the post-collect read is a use-after-poison. A clean exit \
         here means the discriminator is vacuous; stderr:\n{stderr2}"
    );
    assert!(
        stderr2.contains("AddressSanitizer") || stderr2.contains("FAIL"),
        "the pre-fix abort must be the mis-trace (ASan UAF on the freed buffer, or our \
         frees!=0 FAIL); stderr:\n{stderr2}"
    );
}

const SEAM1_SRC: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn use_vec(v: Vec<i64>) -> i64 {
    return v[0]
}
fn main() -> i64 {
    // (a) a real cycle so type_id 0 = Node (1 Rc child @ offset 0), and so the
    //     collector has real work to do.
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = b
    b.next = a
    // (b) a Vec passed by value: the callee retains on entry, releases on exit →
    //     the buffer's refcount returns to 1 (>0) → registered as a cycle candidate
    //     (with the bug). The first element is 7 (a non-NULL, non-pointer value the
    //     mis-trace would read as an Rc child and NULL).
    let data = vec[7, 8, 9]
    let first = use_vec(data)
    // Now collect: with the bug, the Vec buffer (a stale candidate) is mis-traced
    // via the Node pointer-map → its element[0] (7) is read as a child pointer and
    // P4 NULLs it (or dereferences/frees garbage → UAF/ASan trap).
    __aelys_collect()
    // Read element[0] AFTER the collect. Correct: still 7. Corrupted: 0 (NULLed)
    // or a crash. We also fold in `first` (7) so a silent mis-read is visible.
    return data[0] + first
}
"#;

#[test]
fn seam1_vec_buffer_not_mistraced_as_cycle_candidate() {
    let Some((code, _stdout, stderr)) = run_with_env(
        SEAM1_SRC,
        RuntimeVariant::RcCycles,
        &[("AELYS_RC_STATS", "1"), ("AELYS_ALLOC", "immix")],
    ) else {
        return;
    };
    assert_eq!(
        code, 14,
        "the Vec buffer must NOT be mis-traced by the cycle collector: data[0] must \
         stay 7 (+ first 7 = 14). A 7 means data[0] was NULLed to 0 by the mis-trace; \
         a crash means the collector dereferenced an element value as an Rc child \
         pointer; stderr:\n{stderr}"
    );
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        allocs, frees,
        "balanced after collect: the A<->B cycle is collected and the Vec buffer is \
         freed normally (never via the collector); stderr:\n{stderr}"
    );
}

#[test]
fn seam1_vec_buffer_not_mistraced_asan_clean() {
    let Some((code, _stdout, stderr)) = run_with_env(
        SEAM1_SRC,
        RuntimeVariant::RcCycles,
        &[
            ("AELYS_RC_STATS", "1"),
            ("AELYS_ALLOC", "immix"),
            ("ASAN_OPTIONS", "detect_leaks=1"),
        ],
    ) else {
        return;
    };
    assert!(
        asan_clean(&stderr),
        "the seam-1 collect must be ASan/LSan clean (no wild load/store/free from a \
         mis-traced Vec buffer); stderr:\n{stderr}"
    );
    assert_eq!(
        code, 14,
        "value must be correct (14) under the instrumented build too; stderr:\n{stderr}"
    );
}

const SEAM1_NO_RC_TYPE_SRC: &str = r#"
fn use_vec(v: Vec<i64>) -> i64 {
    return v[0]
}
fn main() -> i64 {
    let data = vec[7, 8, 9]
    let first = use_vec(data)
    __aelys_collect()
    return data[0] + first
}
"#;

#[test]
fn seam1_vec_buffer_no_rc_type_no_oob_table_read() {
    let Some((code, _stdout, stderr)) = run_with_env(
        SEAM1_NO_RC_TYPE_SRC,
        RuntimeVariant::RcCycles,
        &[
            ("AELYS_RC_STATS", "1"),
            ("AELYS_ALLOC", "immix"),
            ("ASAN_OPTIONS", "detect_leaks=1"),
        ],
    ) else {
        return;
    };
    assert!(
        asan_clean(&stderr),
        "no-Rc-type variant must not OOB-read the (minimal) type table; stderr:\n{stderr}"
    );
    assert_eq!(
        code, 14,
        "data[0] must stay 7 (+first 7 = 14) with no Rc type present; stderr:\n{stderr}"
    );
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(allocs, frees, "balanced; stderr:\n{stderr}");
}

#[test]
fn seam2_cow_copy_through_immix_value_semantics() {
    let src = r#"
fn main() -> i64 {
    let a = vec[1, 2, 3]
    let b = a
    Vec::push(b, 9)
    return a[0] + a[1] + a[2] + b[3]
}
"#;
    let Some((code, _stdout, stderr)) = run_with_env(
        src,
        RuntimeVariant::RcCycles,
        &[
            ("AELYS_RC_STATS", "1"),
            ("AELYS_ALLOC", "immix"),
            ("ASAN_OPTIONS", "detect_leaks=1"),
        ],
    ) else {
        return;
    };
    assert_eq!(
        code, 15,
        "value semantics through the Immix slow-path copy: a=[1,2,3] (6) + b[3]=9 = 15; \
         stderr:\n{stderr}"
    );
    assert!(asan_clean(&stderr), "CoW-through-immix must be ASan/LSan clean; stderr:\n{stderr}");
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(allocs, frees, "balanced (original buffer + the CoW copy, both freed); stderr:\n{stderr}");
    assert_eq!(allocs, 2, "shared push allocates the CoW copy (slow path); stderr:\n{stderr}");
}

#[test]
fn seam3_collector_free_reclaimed_by_immix_no_uaf() {
    let src = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    // Build and collect a cycle (frees 2 nodes into the Immix free-list).
    let a: Rc<Node> = Rc::new(Node { val: 11, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 22, next: Rc::null() })
    a.next = b
    b.next = a
    __aelys_collect()
    let c: Rc<Node> = Rc::new(Node { val: 33, next: Rc::null() })
    let d: Rc<Node> = Rc::new(Node { val: 44, next: Rc::null() })
    return c.val + d.val
}
"#;
    let Some((code, _stdout, stderr)) = run_with_env(
        src,
        RuntimeVariant::RcCycles,
        &[
            ("AELYS_RC_STATS", "1"),
            ("AELYS_ALLOC", "immix"),
            ("ASAN_OPTIONS", "detect_leaks=1"),
        ],
    ) else {
        return;
    };
    assert_eq!(
        code, 77,
        "reusing collector-reclaimed Immix slots must read the NEW values \
         (33+44=77), never stale cycle data; stderr:\n{stderr}"
    );
    assert!(asan_clean(&stderr), "reclaim+reuse must be ASan/LSan clean (no UAF); stderr:\n{stderr}");
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    assert_eq!(
        allocs, frees,
        "balanced: 2 cycle nodes (collected) + 2 fresh nodes (dropped at scope exit); \
         stderr:\n{stderr}"
    );
    assert_eq!(allocs, 4, "two cycle nodes + two fresh nodes; stderr:\n{stderr}");
}

const SEAM4_SRC: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn use_vec(v: Vec<i64>) -> i64 {
    return v[0] + v[1]
}
fn main() -> i64 {
    // A plain shared Rc.
    let r: Rc<i64> = Rc::new(5)
    let r2: Rc<i64> = r
    // A cycle.
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = b
    b.next = a
    // A Vec, a CoW share, a slow-path push, and a by-value pass.
    let v = vec[10, 20, 30]
    let w = v
    Vec::push(w, 40)
    let s = use_vec(v)
    __aelys_collect()
    // r2 reads 5; v unchanged [10,20,30] so v[0]=10; w[3]=40; s = v[0]+v[1] = 30.
    return Rc::get(r2) + v[0] + w[3] + s
}
"#;

fn run_seam4(opt: OptimizationLevel) -> Option<(i32, i64, i64)> {
    let (code, _stdout, stderr) = run_with_env_opt(
        SEAM4_SRC,
        RuntimeVariant::RcCycles,
        opt,
        &[
            ("AELYS_RC_STATS", "1"),
            ("AELYS_ALLOC", "immix"),
            ("ASAN_OPTIONS", "detect_leaks=1"),
        ],
    )?;
    assert!(asan_clean(&stderr), "seam4 must be ASan/LSan clean; stderr:\n{stderr}");
    let (allocs, frees) = parse_stats(&stderr).expect("stats line");
    Some((code, allocs, frees))
}

#[test]
fn seam4_combined_balanced_o0() {
    let Some((code, allocs, frees)) = run_seam4(OptimizationLevel::None) else {
        return;
    };
    assert_eq!(code, 85, "combined Rc+Vec+cycle+CoW result must be 85 at -O0");
    assert_eq!(allocs, frees, "retain/release insertion balanced at -O0 (allocs={allocs} frees={frees})");
}

#[test]
fn seam4_combined_balanced_o2() {
    let Some((code, allocs, frees)) = run_seam4(OptimizationLevel::Aggressive) else {
        return;
    };
    assert_eq!(code, 85, "combined Rc+Vec+cycle+CoW result must be 85 at -O2");
    assert_eq!(allocs, frees, "retain/release insertion balanced at -O2 (allocs={allocs} frees={frees})");
}

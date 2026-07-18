use aelys_driver::{compile_file_with_llvm_variant, RuntimeVariant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;
use tempfile::tempdir;

fn bench_guard() -> bool {
    std::env::var_os("AELYS_BENCH").is_some()
}

fn record_mode() -> bool {
    std::env::var_os("AELYS_BENCH_RECORD").is_some()
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

fn core_src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("core")
        .join("src")
}

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
    let (a, m) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, m.trim().parse().ok()?))
}

fn build_exe(
    src: &str,
    variant: RuntimeVariant,
    opt: OptimizationLevel,
) -> Option<(tempfile::TempDir, PathBuf)> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(&source_path, opt, false, variant) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                eprintln!("linker unavailable; skipping bench dimension");
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
    Some((dir, exe))
}

fn build_c_probe_o2(dir: &Path, probe_src: &str) -> Option<PathBuf> {
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
    cmd.arg("-O2")
        .arg("-I")
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

#[derive(Debug, Clone)]
struct Stats {
    min: f64,
    median: f64,
    stddev: f64,
    cv: f64,
    cold: f64,
    samples: Vec<f64>,
}

fn compute_stats(cold: f64, mut samples: Vec<f64>) -> Stats {
    assert!(!samples.is_empty(), "need at least one timed sample");
    let n = samples.len() as f64;
    let mean = samples.iter().sum::<f64>() / n;
    let var = samples.iter().map(|x| (x - mean).powi(2)).sum::<f64>() / n;
    let stddev = var.sqrt();
    let cv = if mean > 0.0 { stddev / mean } else { 0.0 };
    samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let min = samples[0];
    let median = samples[samples.len() / 2];
    Stats {
        min,
        median,
        stddev,
        cv,
        cold,
        samples,
    }
}

const WARMUP: usize = 1;
const REPS: usize = 7;
const CV_GATE: f64 = 0.20;

fn time_loop(label: &str, mut f: impl FnMut() -> f64) -> Stats {
    let cold = f();
    let _ = WARMUP;
    let mut samples = Vec::with_capacity(REPS);
    for _ in 0..REPS {
        samples.push(f());
    }
    let stats = compute_stats(cold, samples);
    if stats.cv > CV_GATE {
        panic!(
            "{label}: CV gate exceeded ({:.4} > {CV_GATE}); run too noisy to commit.\n\
             cold={:.3}ms samples(ms)={:?}\n\
             Re-run on a quiesced machine with --test-threads=1.",
            stats.cv, stats.cold, stats.samples
        );
    }
    eprintln!(
        "{label}: median={:.3}ms min={:.3}ms stddev={:.3}ms cv={:.4} cold={:.3}ms samples={:?}",
        stats.median, stats.min, stats.stddev, stats.cv, stats.cold, stats.samples
    );
    stats
}

fn time_loop_ungated(label: &str, mut f: impl FnMut() -> f64) -> Stats {
    let cold = f();
    let mut samples = Vec::with_capacity(REPS);
    for _ in 0..REPS {
        samples.push(f());
    }
    let stats = compute_stats(cold, samples);
    eprintln!(
        "{label} (ungated): min={:.3}ms median={:.3}ms stddev={:.3}ms cv={:.4} cold={:.3}ms samples={:?}",
        stats.min, stats.median, stats.stddev, stats.cv, stats.cold, stats.samples
    );
    stats
}

fn time_run(exe: &Path, env: &[(&str, &str)]) -> (f64, i32, String) {
    let mut cmd = Command::new(exe);
    for (k, v) in env {
        cmd.env(k, v);
    }
    let t = Instant::now();
    let out = cmd.output().expect("run bench exe");
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    let code = out.status.code().expect("process must terminate");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    (ms, code, stderr)
}

const C_ITERS: u64 = 20_000_000;

fn c_probe_src() -> String {
    format!(
        r#"
#include <stdio.h>
#include <stdlib.h>
extern void *aelys_immix_alloc(long long);
extern void aelys_immix_free(void *);
int main(void) {{
    long long N = {C_ITERS};
    volatile long long sink = 0;
    for (long long i = 0; i < N; i++) {{
        char *p = (char *)aelys_immix_alloc(24);
        p[0] = (char)i;
        sink += p[0];
        aelys_immix_free(p);
    }}
    return (int)(sink & 1);
}}
"#
    )
}

const D_COUNT: u64 = 3_000_000;

const D_ORACLE: i32 = 0;

fn amplified_src() -> String {
    format!(
        r#"
struct Node {{ val: i64, next: Rc<Node> }}
fn use_vec(v: Vec<i64>) -> i64 {{
    return v[0] + v[1]
}}
fn main() -> i64 {{
    let mut acc: i64 = 0
    let mut i: i64 = 0
    while i < {D_COUNT} {{
        let r: Rc<i64> = Rc::new(5)
        let r2: Rc<i64> = r
        let a: Rc<Node> = Rc::new(Node {{ val: 1, next: Rc::null() }})
        let b: Rc<Node> = Rc::new(Node {{ val: 2, next: Rc::null() }})
        a.next = b
        b.next = a
        let v = vec[10, 20, 30]
        let w = v
        Vec::push(w, 40)
        let s = use_vec(v)
        __aelys_collect()
        acc = acc + Rc::get(r2) + v[0] + w[3] + s
        i = i + 1
    }}
    return acc % 1000
}}
"#
    )
}

const SPAWN_FLOOR_SRC: &str = r#"
fn main() -> i64 {
    return 0
}
"#;

const RC_DENSE_COUNT: u64 = 120_000;

const RC_DENSE_ORACLE: i32 = 0;

const RC_DENSE_BEFORE_FLOOR_MS: f64 = 200.0;

fn rc_dense_src() -> String {
    format!(
        r#"
fn main() -> i64 {{
    let mut acc: i64 = 0
    let mut i: i64 = 0
    while i < {RC_DENSE_COUNT} {{
        let a: Rc<i64> = Rc::new(7)
        let b: Rc<i64> = a
        let c: Rc<i64> = b
        let d: Rc<i64> = c
        acc = acc + Rc::get(d)
        i = i + 1
    }}
    return acc % 1000
}}
"#
    )
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

fn count_rc_calls(src: &str, opt: OptimizationLevel) -> Option<(usize, usize)> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(&source_path, opt,  true, RuntimeVariant::RcCycles) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                eprintln!("compile unavailable; skipping static count");
                return None;
            }
            panic!("emit_llvm_ir compile should succeed: {err}");
        }
    }
    let mut ll_path = source_path.clone();
    ll_path.set_extension("ll");
    let ir = fs::read_to_string(&ll_path).expect("read emitted .ll");
    let retain = ir.matches("call void @__aelys_rc_retain(").count();
    let release = ir.matches("call void @__aelys_rc_release(").count();
    Some((retain, release))
}

static ELISION_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn count_rc_calls_air_dense(elision: bool) -> (usize, usize) {
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
    fs::write(&source_path, rc_dense_src()).expect("write source");
    let air =
        aelys_driver::lower_file_to_air(&source_path, OptimizationLevel::Aggressive).expect("AIR");
    unsafe {
        std::env::remove_var("AELYS_RC_ELISION");
    }
    use aelys_air::{AirStmtKind, Callee};
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
                    match name.as_str() {
                        "__aelys_rc_retain" => retain += 1,
                        "__aelys_rc_release" => release += 1,
                        _ => {}
                    }
                }
            }
        }
    }
    (retain, release)
}

fn build_dense_exe(elision: bool) -> Option<(tempfile::TempDir, PathBuf)> {
    let _guard = ELISION_ENV_LOCK.lock().unwrap();
    unsafe {
        if elision {
            std::env::remove_var("AELYS_RC_ELISION");
        } else {
            std::env::set_var("AELYS_RC_ELISION", "0");
        }
    }
    let r = build_exe(&rc_dense_src(), RuntimeVariant::RcCycles, OptimizationLevel::Aggressive);
    unsafe {
        std::env::remove_var("AELYS_RC_ELISION");
    }
    r
}

struct CMetrics {
    immix: Stats,
    malloc: Stats,
    ratio_immix_over_malloc: f64,
}

struct DStatic {
    retain_o0: usize,
    release_o0: usize,
    retain_o3: usize,
    release_o3: usize,
}

struct DRuntime {
    stats: Stats,
    allocs: i64,
    frees: i64,
    spawn_floor_ms: f64,
}

struct DDense {
    retain_before: usize,
    release_before: usize,
    retain_after: usize,
    release_after: usize,
    rt_before: Stats,
    rt_after: Stats,
    spawn_floor_ms: f64,
}

fn measure_c() -> Option<CMetrics> {
    let dir = tempdir().expect("tempdir");
    let exe = build_c_probe_o2(dir.path(), &c_probe_src())?;

    let immix = time_loop("C.immix", || {
        let (ms, code, _stderr) = time_run(&exe, &[("AELYS_ALLOC", "immix")]);
        assert!(code == 0 || code == 1, "C probe must terminate cleanly (code={code})");
        ms
    });
    let malloc = time_loop("C.malloc", || {
        let (ms, code, _stderr) = time_run(&exe, &[("AELYS_ALLOC", "malloc")]);
        assert!(code == 0 || code == 1, "C probe must terminate cleanly (code={code})");
        ms
    });
    let ratio = immix.median / malloc.median;
    Some(CMetrics {
        immix,
        malloc,
        ratio_immix_over_malloc: ratio,
    })
}

fn measure_d_static() -> Option<DStatic> {
    let (retain_o0, release_o0) = count_rc_calls(&amplified_src(), OptimizationLevel::None)?;
    let (retain_o3, release_o3) = count_rc_calls(&amplified_src(), OptimizationLevel::Aggressive)?;
    Some(DStatic {
        retain_o0,
        release_o0,
        retain_o3,
        release_o3,
    })
}

fn measure_d_runtime() -> Option<DRuntime> {
    let (_floor_dir, floor_exe) =
        build_exe(SPAWN_FLOOR_SRC, RuntimeVariant::RcCycles, OptimizationLevel::Aggressive)?;
    let floor = time_loop_ungated("D.spawn_floor", || {
        let (ms, code, _stderr) = time_run(&floor_exe, &[("AELYS_ALLOC", "immix")]);
        assert_eq!(code, 0, "spawn-floor empty-main must return 0");
        ms
    });
    let spawn_floor_ms = floor.min;

    let (_dir, exe) =
        build_exe(&amplified_src(), RuntimeVariant::RcCycles, OptimizationLevel::Aggressive)?;
    let (_ms0, code0, stderr0) =
        time_run(&exe, &[("AELYS_ALLOC", "immix"), ("AELYS_RC_STATS", "1")]);
    assert_eq!(code0, D_ORACLE, "amplified workload oracle (acc%1000) must be {D_ORACLE}");
    let (allocs, frees) = parse_stats(&stderr0).expect("[rc] stats line");
    assert_eq!(allocs, frees, "amplified workload must be balanced (allocs={allocs} frees={frees})");

    let stats = time_loop("D.runtime", || {
        let (ms, code, stderr) =
            time_run(&exe, &[("AELYS_ALLOC", "immix"), ("AELYS_RC_STATS", "1")]);
        assert_eq!(code, D_ORACLE, "amplified workload oracle must hold each rep");
        let (a, m) = parse_stats(&stderr).expect("[rc] stats line each rep");
        assert_eq!(a, m, "amplified workload must stay balanced each rep");
        ms
    });

    Some(DRuntime {
        stats,
        allocs,
        frees,
        spawn_floor_ms,
    })
}

fn measure_d_dense() -> Option<DDense> {
    let (retain_before, release_before) = count_rc_calls_air_dense( false);
    let (retain_after, release_after) = count_rc_calls_air_dense( true);

    let (_floor_dir, floor_exe) =
        build_exe(SPAWN_FLOOR_SRC, RuntimeVariant::RcCycles, OptimizationLevel::Aggressive)?;
    let floor = time_loop_ungated("D.dense.spawn_floor", || {
        let (ms, code, _stderr) = time_run(&floor_exe, &[("AELYS_ALLOC", "immix")]);
        assert_eq!(code, 0, "spawn-floor empty-main must return 0");
        ms
    });
    let spawn_floor_ms = floor.min;

    let (_dir_b, exe_b) = build_dense_exe( false)?;
    let (_w, code_b, _) = time_run(&exe_b, &[("AELYS_ALLOC", "immix")]);
    assert_eq!(code_b, RC_DENSE_ORACLE, "dense (before) oracle must be {RC_DENSE_ORACLE}");
    let rt_before = time_loop("D.dense.before(pass-off)", || {
        let (ms, code, _stderr) = time_run(&exe_b, &[("AELYS_ALLOC", "immix")]);
        assert_eq!(code, RC_DENSE_ORACLE, "dense (before) oracle must hold each rep");
        ms
    });

    let (_dir_a, exe_a) = build_dense_exe( true)?;
    let (_w2, code_a, _) = time_run(&exe_a, &[("AELYS_ALLOC", "immix")]);
    assert_eq!(code_a, RC_DENSE_ORACLE, "dense (after) oracle must be {RC_DENSE_ORACLE}");
    let rt_after = time_loop_ungated("D.dense.after(pass-on)", || {
        let (ms, code, _stderr) = time_run(&exe_a, &[("AELYS_ALLOC", "immix")]);
        assert_eq!(code, RC_DENSE_ORACLE, "dense (after) oracle must hold each rep");
        ms
    });

    Some(DDense {
        retain_before,
        release_before,
        retain_after,
        release_after,
        rt_before,
        rt_after,
        spawn_floor_ms,
    })
}

fn artifact_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .join("docs")
        .join("sh1")
        .join("optB_baseline.tsv")
}

fn cpu_model() -> String {
    if let Ok(info) = fs::read_to_string("/proc/cpuinfo") {
        for line in info.lines() {
            if let Some(rest) = line.strip_prefix("model name") {
                if let Some((_, v)) = rest.split_once(':') {
                    return v.trim().to_string();
                }
            }
        }
    }
    if let Ok(out) = Command::new("lscpu").output() {
        let s = String::from_utf8_lossy(&out.stdout);
        for line in s.lines() {
            if let Some(rest) = line.strip_prefix("Model name:") {
                return rest.trim().to_string();
            }
        }
    }
    "unknown".to_string()
}

fn write_artifact(c: &CMetrics, ds: &DStatic, dr: &DRuntime, dd: &DDense) {
    let core_lib = std::env::var("AELYS_CORE_LIB").unwrap_or_else(|_| "<unset>".to_string());
    let mut out = String::new();
    out.push_str("# optB baseline — committed, reproducible benchmark floor (Item B)\n");
    out.push_str("# regenerate with:\n");
    out.push_str("#   cargo build -p aelys-core --release\n");
    out.push_str("#   AELYS_BENCH=1 AELYS_BENCH_RECORD=1 \\\n");
    out.push_str("#     AELYS_CORE_LIB=<abs>/libaelys-core-rc-cycles.a \\\n");
    out.push_str("#     cargo test -p aelys --test bench_harness -- --test-threads=1 --nocapture\n");
    out.push_str("# date                2026-06-18\n");
    out.push_str(&format!("# cpu                 {}\n", cpu_model()));
    out.push_str("# core_archive_profile  release (via AELYS_CORE_LIB)\n");
    out.push_str(&format!("# core_archive_path     {core_lib}\n"));
    out.push_str(&format!("# warmup              {WARMUP}\n"));
    out.push_str(&format!("# reps                {REPS}\n"));
    out.push_str(&format!("# d_count             {D_COUNT}   (amplified outer-loop iterations)\n"));
    out.push_str(&format!("# rc_dense_count      {RC_DENSE_COUNT}   (Item-D dense elision-workload iterations)\n"));
    out.push_str(&format!("# c_iters             {C_ITERS}   (immix alloc/free iterations)\n"));
    out.push_str(&format!("# spawn_floor_ms      {:.3}\n", dr.spawn_floor_ms));
    out.push_str("# NOTE: Dimension C (clang -O2, immix-from-source) and Dimension D\n");
    out.push_str("#       (RELEASE core archive) are at DIFFERENT opt levels and measure\n");
    out.push_str("#       DIFFERENT things. They are NEVER cross-compared.\n");
    out.push_str("# NOTE: warm headline (warm-up discarded); cold first-run noted per dim.\n");
    out.push_str("metric\tvalue\tunit\n");

    let mut row = |m: &str, v: String, u: &str| {
        out.push_str(&format!("{m}\t{v}\t{u}\n"));
    };
    row("c.immix.median_ms", format!("{:.3}", c.immix.median), "ms");
    row("c.malloc.median_ms", format!("{:.3}", c.malloc.median), "ms");
    row("c.ratio_immix_over_malloc", format!("{:.4}", c.ratio_immix_over_malloc), "ratio");
    row("c.immix.cv", format!("{:.4}", c.immix.cv), "ratio");
    row("c.malloc.cv", format!("{:.4}", c.malloc.cv), "ratio");
    row("d.static.retain_o0", format!("{}", ds.retain_o0), "count");
    row("d.static.release_o0", format!("{}", ds.release_o0), "count");
    row("d.static.retain_o3", format!("{}", ds.retain_o3), "count");
    row("d.static.release_o3", format!("{}", ds.release_o3), "count");
    row("d.runtime.median_ms", format!("{:.3}", dr.stats.median), "ms");
    row("d.runtime.cv", format!("{:.4}", dr.stats.cv), "ratio");
    row("d.runtime.allocs", format!("{}", dr.allocs), "count");
    row("d.runtime.frees", format!("{}", dr.frees), "count");
    let combined_before = dd.retain_before + dd.release_before;
    let combined_after = dd.retain_after + dd.release_after;
    let rt_before_net = (dd.rt_before.median - dd.spawn_floor_ms).max(0.0);
    let rt_after_net = (dd.rt_after.median - dd.spawn_floor_ms).max(0.0);
    let delta_ms = rt_before_net - rt_after_net;
    let delta_pct = if rt_before_net > 0.0 {
        100.0 * delta_ms / rt_before_net
    } else {
        0.0
    };
    row("d.dense.static.combined_before", format!("{combined_before}"), "count");
    row("d.dense.static.combined_after", format!("{combined_after}"), "count");
    row("d.dense.static.retain_before", format!("{}", dd.retain_before), "count");
    row("d.dense.static.release_before", format!("{}", dd.release_before), "count");
    row("d.dense.static.retain_after", format!("{}", dd.retain_after), "count");
    row("d.dense.static.release_after", format!("{}", dd.release_after), "count");
    row("d.dense.runtime.before_median_ms", format!("{:.3}", dd.rt_before.median), "ms");
    row("d.dense.runtime.before_cv", format!("{:.4}", dd.rt_before.cv), "ratio");
    row("d.dense.runtime.after_median_ms", format!("{:.3}", dd.rt_after.median), "ms");
    row("d.dense.runtime.after_cv", format!("{:.4}", dd.rt_after.cv), "ratio");
    row("d.dense.runtime.before_net_ms", format!("{rt_before_net:.3}"), "ms");
    row("d.dense.runtime.after_net_ms", format!("{rt_after_net:.3}"), "ms");
    row("d.dense.runtime.delta_ms", format!("{delta_ms:.3}"), "ms");
    row("d.dense.runtime.delta_pct", format!("{delta_pct:.2}"), "percent");

    fs::write(artifact_path(), out).expect("write optB_baseline.tsv");
    eprintln!("wrote {}", artifact_path().display());
}

fn parse_artifact() -> std::collections::BTreeMap<String, (String, String)> {
    let text = fs::read_to_string(artifact_path())
        .expect("optB_baseline.tsv must exist (generate with AELYS_BENCH_RECORD=1)");
    let mut map = std::collections::BTreeMap::new();
    for line in text.lines() {
        if line.starts_with('#') || line.trim().is_empty() {
            continue;
        }
        if line.starts_with("metric\t") {
            continue;
        }
        let mut parts = line.split('\t');
        let metric = parts.next().expect("metric column").to_string();
        let value = parts.next().expect("value column").to_string();
        let unit = parts.next().expect("unit column").to_string();
        map.insert(metric, (value, unit));
    }
    map
}

const EXPECTED_METRICS: &[&str] = &[
    "c.immix.median_ms",
    "c.malloc.median_ms",
    "c.ratio_immix_over_malloc",
    "c.immix.cv",
    "c.malloc.cv",
    "d.static.retain_o0",
    "d.static.release_o0",
    "d.static.retain_o3",
    "d.static.release_o3",
    "d.runtime.median_ms",
    "d.runtime.cv",
    "d.runtime.allocs",
    "d.runtime.frees",
    "d.dense.static.combined_before",
    "d.dense.static.combined_after",
    "d.dense.runtime.before_median_ms",
    "d.dense.runtime.after_median_ms",
    "d.dense.runtime.delta_ms",
    "d.dense.runtime.delta_pct",
];

#[test]
fn bench_c_alloc_throughput() {
    if !bench_guard() {
        return;
    }
    let Some(c) = measure_c() else {
        return;
    };
    eprintln!(
        "C: immix.median={:.3}ms malloc.median={:.3}ms ratio(immix/malloc)={:.4}",
        c.immix.median, c.malloc.median, c.ratio_immix_over_malloc
    );
}

#[test]
fn bench_d_static_retain_release() {
    if !bench_guard() {
        return;
    }
    let Some(ds) = measure_d_static() else {
        return;
    };
    eprintln!(
        "D.static (amplified): retain o0={} o3={} | release o0={} o3={}",
        ds.retain_o0, ds.retain_o3, ds.release_o0, ds.release_o3
    );
    assert!(
        ds.retain_o0 >= 1 && ds.release_o0 >= 1,
        "D-static counts must be > 0 (got retain={} release={}); IR-emit or matcher broke",
        ds.retain_o0, ds.release_o0
    );
    assert_eq!(ds.retain_o0, 3, "baseline retain count drifted (-O0, pass off)");
    assert_eq!(ds.release_o0, 6, "baseline release count drifted (-O0, pass off)");
    assert_eq!(ds.retain_o3, 2, "Item-D elision: retain must be 2 at -O3 (was 3)");
    assert_eq!(ds.release_o3, 5, "Item-D elision: release must be 5 at -O3 (was 6)");
    assert_eq!(
        ds.retain_o0 - ds.retain_o3,
        1,
        "Item-D must elide exactly one retain at -O3 (the let r2 = r clone)"
    );
    assert_eq!(
        ds.release_o0 - ds.release_o3,
        1,
        "Item-D must elide exactly one release at -O3 (the release of r)"
    );
    if let Some((s_retain, s_release)) = count_rc_calls(SEAM4_SRC, OptimizationLevel::None) {
        assert_eq!(
            s_retain, ds.retain_o0,
            "SEAM4 and amplified must emit identical retain call sites at -O0 (loop is a backedge)"
        );
        assert_eq!(
            s_release, ds.release_o0,
            "SEAM4 and amplified must emit identical release call sites at -O0 (loop is a backedge)"
        );
    }
    if let Some((s_retain, s_release)) = count_rc_calls(SEAM4_SRC, OptimizationLevel::Aggressive) {
        assert_eq!(
            (s_retain, s_release),
            (ds.retain_o3, ds.release_o3),
            "SEAM4 and amplified must share the elided -O3 counts (2/5)"
        );
    }
}

#[test]
fn bench_d_runtime_amplified() {
    if !bench_guard() {
        return;
    }
    let Some(dr) = measure_d_runtime() else {
        return;
    };
    let spawn_frac = if dr.stats.median > 0.0 {
        dr.spawn_floor_ms / dr.stats.median
    } else {
        0.0
    };
    eprintln!(
        "D.runtime: median={:.3}ms cv={:.4} allocs={} frees={} | spawn_floor={:.3}ms ({:.2}% of median)",
        dr.stats.median,
        dr.stats.cv,
        dr.allocs,
        dr.frees,
        dr.spawn_floor_ms,
        spawn_frac * 100.0
    );
    assert!(
        dr.stats.median >= 200.0,
        "D.runtime median ({:.1}ms) below 200ms floor — increase D_COUNT",
        dr.stats.median
    );
    assert!(
        spawn_frac < 0.03,
        "spawn floor ({:.3}ms) is {:.2}% of median ({:.1}ms) — exceeds ~3%; increase D_COUNT",
        dr.spawn_floor_ms,
        spawn_frac * 100.0,
        dr.stats.median
    );
}

#[test]
fn bench_d_runtime_dense() {
    if !bench_guard() {
        return;
    }
    let Some(dd) = measure_d_dense() else {
        return;
    };
    let combined_before = dd.retain_before + dd.release_before;
    let combined_after = dd.retain_after + dd.release_after;
    let before_net = (dd.rt_before.median - dd.spawn_floor_ms).max(0.0);
    let after_net = (dd.rt_after.median - dd.spawn_floor_ms).max(0.0);
    eprintln!(
        "D.dense STATIC per-iter: retain {}->{} release {}->{} (combined {}->{}) | \
         RUNTIME before={:.3}ms (cv={:.4}) after={:.3}ms (cv={:.4}) | net before={:.3}ms \
         after={:.3}ms delta={:.3}ms ({:.2}%) | spawn_floor={:.3}ms",
        dd.retain_before, dd.retain_after, dd.release_before, dd.release_after,
        combined_before, combined_after,
        dd.rt_before.median, dd.rt_before.cv, dd.rt_after.median, dd.rt_after.cv,
        before_net, after_net, before_net - after_net,
        if before_net > 0.0 { 100.0 * (before_net - after_net) / before_net } else { 0.0 },
        dd.spawn_floor_ms,
    );
    assert_eq!(combined_before, 7, "dense static combined-before must be 7 (3 retain + 4 release)");
    assert_eq!(combined_after, 1, "dense static combined-after must be 1 (0 retain + 1 release)");
    assert!(
        dd.rt_before.median >= RC_DENSE_BEFORE_FLOOR_MS,
        "dense BEFORE median ({:.1}ms) below {RC_DENSE_BEFORE_FLOOR_MS}ms floor — increase RC_DENSE_COUNT",
        dd.rt_before.median
    );
    assert!(
        after_net < before_net,
        "Item-D dense: after-median ({after_net:.3}ms net) must be < before ({before_net:.3}ms net) \
         — no measurable runtime gain means PARK (design §7/§9)"
    );
    let speedup = if after_net > 0.0 { before_net / after_net } else { f64::INFINITY };
    eprintln!("D.dense MEASURED speedup (before/after, net): {speedup:.0}x");
}

#[test]
fn bench_record_baseline() {
    if !bench_guard() {
        return;
    }
    if record_mode() {
        let (Some(c), Some(ds), Some(dr), Some(dd)) = (
            measure_c(),
            measure_d_static(),
            measure_d_runtime(),
            measure_d_dense(),
        ) else {
            eprintln!("a dimension was unavailable (linker/clang); skipping artifact write");
            return;
        };
        write_artifact(&c, &ds, &dr, &dd);
        return;
    }
    let map = parse_artifact();
    for m in EXPECTED_METRICS {
        assert!(
            map.contains_key(*m),
            "optB_baseline.tsv missing expected metric row: {m}"
        );
    }
    eprintln!(
        "optB_baseline.tsv parsed OK ({} metric rows present)",
        EXPECTED_METRICS.len()
    );
}

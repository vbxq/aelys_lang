use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Once;
use std::time::{Duration, Instant};
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const REJECT_LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
];

const ALLOCATORS: &[(&str, Option<&str>)] = &[("immix", None), ("malloc", Some("malloc"))];

static WARM: Once = Once::new();

// exactly once and its spurious e0901 cannot be attributed to a fixture
fn warm_core_archive() {
    WARM.call_once(|| {
        let Ok(dir) = tempdir() else { return };
        let path = dir.path().join("warmup.aelys");
        if fs::write(&path, "fn main() -> i64 { return 0 }\n").is_err() {
            return;
        }
        let _ = compile_file_with_llvm_variant(
            &path,
            OptimizationLevel::None,
            false,
            RuntimeVariant::Rc,
        );
    });
}

struct Outcome {
    exit: i32,
    stdout: String,
    stderr: String,
    stats: Option<(i64, i64)>,
    timed_out: bool,
}

struct Harness {
    dir: TempDir,
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

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
    let (a, m) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, m.trim().parse().ok()?))
}

fn exit_code(status: &std::process::ExitStatus) -> i32 {
    if let Some(code) = status.code() {
        return code;
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(signal) = status.signal() {
            return 128 + signal;
        }
    }
    -1
}

fn slug(id: &str, tag: &str) -> String {
    let mut s = String::with_capacity(id.len() + tag.len() + 1);
    for c in id.chars().chain(std::iter::once('_')).chain(tag.chars()) {
        s.push(if c.is_ascii_alphanumeric() { c } else { '_' });
    }
    s
}

impl Harness {
    fn new() -> Self {
        warm_core_archive();
        Harness {
            dir: tempdir().expect("tempdir"),
        }
    }

    fn write(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let path = self.dir.path().join(format!("{}.aelys", slug(id, tag)));
        fs::write(&path, src).expect("write fixture");
        path
    }

    // none means the toolchain cannot link here, which is a skip and not a failure
    fn compile(&self, id: &str, tag: &str, src: &str, opt: OptimizationLevel) -> Option<PathBuf> {
        let path = self.write(id, tag, src);
        match compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::Rc) {
            Ok(()) => {}
            Err(err) => {
                if linker_unavailable(&err.to_string()) {
                    return None;
                }
                panic!("{id} at {tag} must compile:\n{src}\nerror: {err}");
            }
        }
        let exe = exe_path_for(&path);
        exe.is_file().then_some(exe)
    }

    fn run(&self, exe: &Path, alloc: Option<&str>) -> Outcome {
        self.run_timed(exe, alloc, None)
    }

    fn run_timed(&self, exe: &Path, alloc: Option<&str>, deadline: Option<Duration>) -> Outcome {
        let mut cmd = Command::new(exe);
        cmd.env("AELYS_RC_STATS", "1");
        if let Some(a) = alloc {
            cmd.env("AELYS_ALLOC", a);
        }
        let Some(deadline) = deadline else {
            let out = cmd.output().expect("run exe");
            let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
            return Outcome {
                exit: exit_code(&out.status),
                stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
                stats: parse_stats(&stderr),
                stderr,
                timed_out: false,
            };
        };
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let mut child = cmd.spawn().expect("spawn exe");
        let start = Instant::now();
        loop {
            match child.try_wait().expect("try_wait") {
                Some(_) => break,
                None => {
                    if start.elapsed() >= deadline {
                        let _ = child.kill();
                        let _ = child.wait();
                        return Outcome {
                            exit: -1,
                            stdout: String::new(),
                            stats: None,
                            stderr: String::new(),
                            timed_out: true,
                        };
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
            }
        }
        let out = child.wait_with_output().expect("wait_with_output");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Outcome {
            exit: exit_code(&out.status),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stats: parse_stats(&stderr),
            stderr,
            timed_out: false,
        }
    }

    fn reject(&self, id: &str, tag: &str, src: &str, opt: OptimizationLevel) -> String {
        let path = self.write(id, tag, src);
        match lower_file_to_air(&path, opt) {
            Ok(_) => panic!("{id} at {tag} must be rejected, but it was accepted:\n{src}"),
            Err(rendered) => rendered,
        }
    }

    fn accepts(&self, id: &str, tag: &str, src: &str, opt: OptimizationLevel) {
        let path = self.write(id, tag, src);
        if let Err(rendered) = lower_file_to_air(&path, opt) {
            panic!("{id} at {tag} must be accepted:\n{src}\nrejected with:\n{rendered}");
        }
    }
}

#[derive(Clone, Copy)]
enum Memory {
    Ignore,
    Balanced,
    NamedLeak(&'static str),
}

#[derive(Clone, Copy)]
enum Oracle {
    Exit(i32),
    ExitOut(i32, &'static str),
    ExitOutErr(i32, &'static str, &'static str),
    Terminates(u64, i32, &'static str),
    // cannot express it, because the owed value is an aslr-varying address.
    AllocAgree(i32),
    // an exact (allocs, frees) pair beside the value. balanced and namedleak both pass on
    ExitOutStats(i32, &'static str, i64, i64),
    Balanced(i32),
    Leaks(i32, &'static str),
    AllocsAt(&'static str, OptimizationLevel, i32, i64),
}

fn check(h: &Harness, id: &str, src: &str, exit: i32, out: Option<&str>, mem: Memory) {
    for (name, opt) in LEVELS {
        let Some(exe) = h.compile(id, name, src, *opt) else {
            eprintln!("{id}: linker unavailable, skipping");
            return;
        };
        for (alloc_name, alloc) in ALLOCATORS {
            let o = h.run(&exe, *alloc);
            assert_eq!(
                o.exit, exit,
                "{id} at {name} under {alloc_name}: the answer MUST be {exit}\n\
                 source:{src}\nstdout: {:?}\nstderr:\n{}",
                o.stdout, o.stderr
            );
            if let Some(expected) = out {
                assert_eq!(
                    o.stdout, expected,
                    "{id} at {name} under {alloc_name}: stdout MUST be {expected:?}\nsource:{src}"
                );
            }
            match mem {
                Memory::Ignore => {}
                Memory::Balanced => {
                    let (allocs, frees) = o.stats.unwrap_or_else(|| {
                        panic!("{id} at {name}/{alloc_name}: no [rc] stats line")
                    });
                    assert_eq!(
                        allocs, frees,
                        "{id} at {name}/{alloc_name}: every buffer freed exactly once \
                         (allocs={allocs} frees={frees})\nsource:{src}"
                    );
                }
                Memory::NamedLeak(why) => {
                    let (allocs, frees) = o.stats.unwrap_or_else(|| {
                        panic!("{id} at {name}/{alloc_name}: no [rc] stats line")
                    });
                    assert!(
                        frees < allocs,
                        "{id} at {name}/{alloc_name}: {why} (allocs={allocs} frees={frees})"
                    );
                }
            }
        }
    }
}

fn check_allocs(
    h: &Harness,
    id: &str,
    src: &str,
    level: &str,
    opt: OptimizationLevel,
    exit: i32,
    n: i64,
) {
    let Some(exe) = h.compile(id, level, src, opt) else {
        eprintln!("{id}: linker unavailable, skipping");
        return;
    };
    let o = h.run(&exe, None);
    assert_eq!(
        o.exit, exit,
        "{id} at {level}: the answer MUST be {exit}\nstderr:\n{}",
        o.stderr
    );
    let (allocs, _) = o
        .stats
        .unwrap_or_else(|| panic!("{id} at {level}: no [rc] stats line"));
    assert_eq!(
        allocs, n,
        "{id} at {level}: MUST allocate exactly {n} time(s)\nstderr:\n{}",
        o.stderr
    );
}

fn run_row(h: &Harness, id: &str, src: &str, oracle: Oracle) {
    match oracle {
        Oracle::Exit(e) => check(h, id, src, e, None, Memory::Ignore),
        Oracle::ExitOut(e, out) => check(h, id, src, e, Some(out), Memory::Ignore),
        Oracle::Balanced(e) => check(h, id, src, e, None, Memory::Balanced),
        Oracle::Leaks(e, why) => check(h, id, src, e, None, Memory::NamedLeak(why)),
        Oracle::AllocsAt(name, opt, e, n) => check_allocs(h, id, src, name, opt, e, n),
        Oracle::ExitOutErr(e, out, err) => check_stderr(h, id, src, e, out, err),
        Oracle::Terminates(ms, e, out) => check_terminates(h, id, src, ms, e, out),
        Oracle::AllocAgree(e) => check_alloc_agree(h, id, src, e),
        Oracle::ExitOutStats(e, out, a, f) => check_stats(h, id, src, e, out, a, f),
    }
}

fn check_stats(h: &Harness, id: &str, src: &str, exit: i32, out: &str, allocs: i64, frees: i64) {
    for (name, opt) in LEVELS {
        let Some(exe) = h.compile(id, name, src, *opt) else {
            eprintln!("{id}: linker unavailable, skipping");
            return;
        };
        for (alloc_name, alloc) in ALLOCATORS {
            let o = h.run(&exe, *alloc);
            assert_eq!(
                o.exit, exit,
                "{id} at {name}/{alloc_name}: the answer MUST be {exit}\nsource:{src}\n\
                 stdout: {:?}\nstderr:\n{}",
                o.stdout, o.stderr
            );
            assert_eq!(
                o.stdout, out,
                "{id} at {name}/{alloc_name}: stdout MUST be {out:?}\nsource:{src}"
            );
            let (a, f) = o
                .stats
                .unwrap_or_else(|| panic!("{id} at {name}/{alloc_name}: no [rc] stats line"));
            assert_eq!(
                (a, f),
                (allocs, frees),
                "{id} at {name}/{alloc_name}: MUST be exactly allocs={allocs} frees={frees}, \
                 got allocs={a} frees={f}\nsource:{src}"
            );
        }
    }
}

fn check_stderr(h: &Harness, id: &str, src: &str, exit: i32, out: &str, err_sub: &str) {
    for (name, opt) in LEVELS {
        let Some(exe) = h.compile(id, name, src, *opt) else {
            eprintln!("{id}: linker unavailable, skipping");
            return;
        };
        for (alloc_name, alloc) in ALLOCATORS {
            let o = h.run(&exe, *alloc);
            assert_eq!(
                o.exit, exit,
                "{id} at {name}/{alloc_name}: the answer MUST be {exit}\nsource:{src}\n\
                 stdout: {:?}\nstderr:\n{}",
                o.stdout, o.stderr
            );
            assert_eq!(
                o.stdout, out,
                "{id} at {name}/{alloc_name}: stdout MUST be {out:?}\nsource:{src}"
            );
            assert!(
                o.stderr.contains(err_sub),
                "{id} at {name}/{alloc_name}: stderr MUST contain {err_sub:?}\nsource:{src}\n\
                 stderr:\n{}",
                o.stderr
            );
        }
    }
}

fn check_terminates(h: &Harness, id: &str, src: &str, ms: u64, exit: i32, out: &str) {
    let deadline = Duration::from_millis(ms);
    for (name, opt) in LEVELS {
        let Some(exe) = h.compile(id, name, src, *opt) else {
            eprintln!("{id}: linker unavailable, skipping");
            return;
        };
        for (alloc_name, alloc) in ALLOCATORS {
            let o = h.run_timed(&exe, *alloc, Some(deadline));
            assert!(
                !o.timed_out,
                "{id} at {name}/{alloc_name}: MUST terminate within {ms}ms; it did not, which is \
                 the compile-clean hang a lost write in a loop condition produces\nsource:{src}"
            );
            assert_eq!(
                o.exit, exit,
                "{id} at {name}/{alloc_name}: the answer MUST be {exit}\nsource:{src}\n\
                 stderr:\n{}",
                o.stderr
            );
            assert_eq!(
                o.stdout, out,
                "{id} at {name}/{alloc_name}: stdout MUST be {out:?}\nsource:{src}"
            );
        }
    }
}

fn check_alloc_agree(h: &Harness, id: &str, src: &str, exit: i32) {
    for (name, opt) in LEVELS {
        let Some(exe) = h.compile(id, name, src, *opt) else {
            eprintln!("{id}: linker unavailable, skipping");
            return;
        };
        let mut legs: Vec<(&str, Outcome)> = Vec::new();
        for (alloc_name, alloc) in ALLOCATORS {
            legs.push((alloc_name, h.run(&exe, *alloc)));
        }
        for (alloc_name, o) in &legs {
            assert_eq!(
                o.exit, exit,
                "{id} at {name}/{alloc_name}: the answer MUST be {exit}\nsource:{src}\n\
                 stderr:\n{}",
                o.stderr
            );
        }
        let (a_name, a) = &legs[0];
        for (b_name, b) in &legs[1..] {
            assert_eq!(
                a.stdout, b.stdout,
                "{id} at {name}: the {a_name} and {b_name} legs MUST agree with each other; no \
                 absolute value is asserted because the owed value varies with ASLR\nsource:{src}"
            );
        }
    }
}

fn run_rows(h: &Harness, rows: &[(&str, &str, Oracle)]) {
    for (id, src, oracle) in rows {
        run_row(h, id, src, *oracle);
    }
}

fn run_rejects(h: &Harness, rows: &[(&str, &str, &str)]) {
    for (id, code, src) in rows {
        for (name, opt) in REJECT_LEVELS {
            let rendered = h.reject(id, name, src, *opt);
            assert!(
                rendered.contains(&format!("[{code}]")),
                "{id} at {name} MUST be rejected with {code}, got:\n{rendered}"
            );
        }
    }
}

// completeness argument: the table is organized by the mutation entry-point enumeration in forensics
// compiler-side entry point that reaches a store is a row here: plain index assign, compound assign,
// on a shared buffer, self assign, and a call that transfers a share. the deferred entry points
// (nested vec, vec in an array or a struct, a slice, for-each) fail closed and are pinned in group x.

const GROUP_V: &[(&str, &str, Oracle)] = &[
    (
        "SI-V17",
        r#"
struct Cell { x: i64 }
fn read(p: &i64) -> i64 { return *p }
fn main() -> i64 {
    let r = Rc::new(Cell{x: 7})
    let p = &Rc::get(r).x
    let q = Rc::new(Cell{x: 31})
    return read(p) + Rc::get(q).x
}
"#,
        Oracle::Balanced(38),
    ),
    (
        "SI-V01",
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let w = v
    v[0] = 9
    return w[0]
}
"#,
        Oracle::Balanced(1),
    ),
    (
        "SI-V02",
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let mut w = vec[7, 7, 7]
    w = v
    v[0] = 9
    return w[0]
}
"#,
        Oracle::Balanced(1),
    ),
    (
        "SI-V03",
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let w = v
    v[0] += 8
    return w[0]
}
"#,
        Oracle::Balanced(1),
    ),
    (
        "SI-V04",
        r#"
fn poke(mut x: Vec<i64>) -> i64 {
    x[0] = 9
    return x[0]
}
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let _ = poke(v)
    return v[0]
}
"#,
        Oracle::Balanced(1),
    ),
    (
        "SI-V05",
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let w = v
    let mut i = 0
    while i < 3 {
        v[i] = 9
        i = i + 1
    }
    return w[0] + w[1] + w[2]
}
"#,
        Oracle::Balanced(6),
    ),
    (
        // the env slot owns a share, so the closure write detaches; the env leak is by design
        "SI-V06",
        r#"
fn make() -> fn() -> i64 {
    let mut v = vec[1, 2, 3]
    let w = v
    let f = fn() -> i64 {
        v[0] = 9
        return w[0]
    }
    return f
}
fn main() -> i64 {
    let g = make()
    let junk = vec[77, 77, 77]
    return g()
}
"#,
        Oracle::Leaks(
            1,
            "the captured buffer leaks with the deliberately leaked closure env",
        ),
    ),
    (
        "SI-V07",
        r#"
fn main() -> i64 {
    let a = vec[1, 2, 3]
    let b = a
    Vec::push(b, 9)
    return a[0] + a[1] + a[2] + b[0] + b[1] + b[2] + b[3]
}
"#,
        Oracle::Balanced(21),
    ),
    (
        "SI-V08",
        r#"
fn main() -> i64 {
    let a = [1, 2, 3]
    let mut b = a
    b[0] = 9
    return a[0]
}
"#,
        Oracle::Exit(1),
    ),
    (
        // both directions of the aliasing relation, so a fix that detaches the wrong side is caught
        "SI-V09",
        r#"
fn main() -> i64 {
    let mut a = vec[1, 2, 3]
    let b = a
    a[0] = 9
    let mut c = vec[4, 5, 6]
    let d = c
    c[1] = 8
    return a[0] + b[0] + c[1] + d[1]
}
"#,
        Oracle::Balanced(23),
    ),
    (
        // stale-but-correct read is impossible. this row must never return 77.
        "SI-V10",
        r#"
fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    Vec::push(v, 2)
    Vec::push(v, 3)
    let w = v
    Vec::push(v, 4)
    Vec::push(v, 5)
    let junk = vec[77, 77, 77]
    return w[0]
}
"#,
        Oracle::Balanced(1),
    ),
    (
        "SI-V11",
        r#"
fn main() -> i64 {
    let mut v = vec[7, 2, 3]
    v = v
    v[0] = v[0]
    return v[0]
}
"#,
        Oracle::Balanced(7),
    ),
    (
        "SI-V12",
        r#"
fn id(x: Vec<i64>) -> Vec<i64> { return x }
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let w = id(v)
    return w[0] + v[0]
}
"#,
        Oracle::Balanced(2),
    ),
    (
        "SI-V13",
        r#"
fn main() -> i64 {
    let mut v = vec[0, 0, 0, 0]
    let mut i = 0
    while i < 1000000 {
        v[i % 4] = i
        i = i + 1
    }
    return v[0]
}
"#,
        Oracle::AllocsAt("-O0", OptimizationLevel::None, 60, 1),
    ),
    (
        "SI-V14",
        r#"
fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    Vec::push(v, 2)
    Vec::push(v, 3)
    let mut w = Vec::new()
    w = v
    Vec::push(w, 4)
    w[0] = 9
    let junk = vec[77, 77, 77]
    return v[0]
}
"#,
        Oracle::Balanced(1),
    ),
    (
        "SI-V15",
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let mut acc = 0
    if v[2] == 3 {
        let w = v
        v[0] = 9
        let junk = vec[77, 77, 77]
        acc = w[0]
    }
    return acc
}
"#,
        Oracle::Balanced(1),
    ),
    (
        "SI-V16",
        r#"
fn grow(mut x: Vec<i64>) -> i64 {
    Vec::push(x, 4)
    Vec::push(x, 5)
    x[0] = 9
    return x[0]
}
fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    Vec::push(v, 2)
    Vec::push(v, 3)
    let _ = grow(v)
    let junk = vec[77, 77, 77]
    return v[0]
}
"#,
        Oracle::Balanced(1),
    ),
];

#[test]
fn group_v_vec_value_semantics() {
    let h = Harness::new();
    run_rows(&h, GROUP_V);
}

const GROUP_S: &[(&str, &str, Oracle)] = &[
    (
        "SI-S01",
        r#"
struct P { x: i64, y: i64 }
fn main() -> i64 {
    let mut a = P{x: 1, y: 2}
    let b = a
    a.x = 9
    return b.x
}
"#,
        Oracle::Exit(1),
    ),
    (
        "SI-S02",
        r#"
struct P { x: i64, y: i64 }
fn poke(mut p: P) -> i64 {
    p.x = 9
    return p.x
}
fn main() -> i64 {
    let mut a = P{x: 1, y: 2}
    let _ = poke(a)
    return a.x
}
"#,
        Oracle::Exit(1),
    ),
    (
        "SI-S03",
        r#"
struct Box { cells: [i64; 3] }
fn main() -> i64 {
    let mut a = Box{cells: [1, 2, 3]}
    let b = a
    a.cells[0] = 9
    return b.cells[0]
}
"#,
        Oracle::Exit(1),
    ),
    (
        "SI-T01",
        r#"
fn main() -> i64 {
    let s = "hello"
    let t = s
    return t.len
}
"#,
        Oracle::Exit(5),
    ),
    (
        "SI-T02",
        r#"
fn main() -> i64 {
    let a = "abc"
    let b = a + "de"
    return a.len + b.len
}
"#,
        Oracle::Exit(8),
    ),
];

const GROUP_T_REJECTS: &[(&str, &str, &str)] = &[(
    "SI-T03",
    "E0304",
    r#"
fn main() -> i64 {
    let mut s = "hello"
    s[0] = "z"
    return 0
}
"#,
)];

#[test]
fn group_st_struct_and_string_value_semantics() {
    let h = Harness::new();
    run_rows(&h, GROUP_S);
    run_rejects(&h, GROUP_T_REJECTS);
}

// ============================== groups m and b: moves and borrows ==============================

const GROUP_M_REJECTS: &[(&str, &str, &str)] = &[
    (
        "SI-M01",
        "E0701",
        r#"
struct Resource { id: i64 }
fn main() -> i64 {
    let a = Resource{id: 1}
    let b = a
    return a.id
}
"#,
    ),
    (
        "SI-M02",
        "E0702",
        r#"
struct Resource { id: i64 }
fn main() -> i64 {
    let a = Resource{id: 1}
    let b = a
    let c = a
    return 0
}
"#,
    ),
    (
        "SI-M03",
        "E0703",
        r#"
struct Resource { id: i64 }
fn main() -> i64 {
    let cond = true
    let a = Resource{id: 1}
    if cond {
        let b = a
    }
    return a.id
}
"#,
    ),
    (
        "SI-M04",
        "E0712",
        r#"
struct Resource { id: i64 }
fn touch(r: &Resource) {
}
fn main() -> i64 {
    let a = Resource{id: 1}
    let r = &a
    let b = a
    touch(r)
    return 0
}
"#,
    ),
    (
        "SI-M05",
        "E0702",
        r#"
struct Resource { id: i64 }
fn main() -> i64 {
    let a = Resource{id: 1}
    if false {
        let b = a
        let c = a
    }
    return 0
}
"#,
    ),
];

const GROUP_M_SCAFFOLD: &[(&str, &str, Oracle)] = &[(
    "SI-M06",
    r#"
struct Handle { id: i64 }
fn main() -> i64 {
    let a = Handle{id: 1}
    let b = a
    let c = a
    return a.id
}
"#,
    Oracle::Exit(1),
)];

const GROUP_B_REJECTS: &[(&str, &str, &str)] = &[
    (
        "SI-B01",
        "E0711",
        r#"
fn main() -> i64 {
    let mut x = 3
    let r = &x
    x = 5
    return *r
}
"#,
    ),
    (
        "SI-B02",
        "E0713",
        r#"
fn main() -> i64 {
    let mut x = 3
    let r1 = &mut x
    let r2 = &mut x
    *r1 = 5
    *r2 = 6
    return x
}
"#,
    ),
    (
        "SI-B03",
        "E0711",
        r#"
fn main() -> i64 {
    let v = vec[10, 20, 30]
    let r = &v[0]
    Vec::push(v, 40)
    return *r
}
"#,
    ),
    (
        "SI-B04",
        "E0711",
        r#"
fn main() -> i64 {
    let mut x = 3
    let r = &x
    x = 5
    if false {
        return *r
    }
    return 0
}
"#,
    ),
];

// the anti-vacuity legs: a checker that rejected everything would satisfy si-b01 to si-b04
const GROUP_B_ACCEPTS: &[(&str, &str, Oracle)] = &[
    (
        "SI-B05",
        r#"
fn main() -> i64 {
    let mut x = 3
    let r1 = &mut x
    *r1 = 5
    let r2 = &mut x
    *r2 = 6
    return x
}
"#,
        Oracle::Exit(6),
    ),
    (
        "SI-B06",
        r#"
struct Resource { id: i64 }
fn touch(r: &Resource) {
}
fn main() -> i64 {
    let a = Resource{id: 1}
    let r = &a
    touch(r)
    let b = a
    return b.id
}
"#,
        Oracle::Exit(1),
    ),
    (
        "SI-B07",
        r#"
fn main() -> i64 {
    let mut x = 3
    let r = &mut x
    *r = 9
    return x
}
"#,
        Oracle::Exit(9),
    ),
    (
        // `&mut *r` lowered to the address of the loaded pointee, so the write
        "SI-B08",
        r#"
fn f(r: &mut i64) {
    let r2 = &mut *r
    *r2 = 5
}
fn main() -> i64 {
    let mut x = 0
    f(&mut x)
    return x
}
"#,
        Oracle::Exit(5),
    ),
    (
        // the same reborrow taken twice, so a fix that collapses one level but not the chain is caught
        "SI-B09",
        r#"
fn main() -> i64 {
    let mut x = 1
    let r = &mut x
    let q = &mut *r
    let p = &mut *q
    *p = 9
    return x
}
"#,
        Oracle::Exit(9),
    ),
];

#[test]
fn group_mb_moves_and_borrows() {
    let h = Harness::new();
    run_rejects(&h, GROUP_M_REJECTS);
    run_rows(&h, GROUP_M_SCAFFOLD);
    run_rejects(&h, GROUP_B_REJECTS);
    run_rows(&h, GROUP_B_ACCEPTS);
}

const GROUP_R: &[(&str, &str, Oracle)] = &[
    (
        "SI-R01",
        r#"
fn f(c: bool) -> i64 {
    let r = Rc::new(5)
    if c {
        return Rc::get(r)
    }
    return 0
}
fn main() -> i64 {
    return f(true)
}
"#,
        Oracle::Balanced(5),
    ),
    (
        "SI-R02",
        r#"
enum Result<T, E> { Ok(T), Err(E) }
fn probe(x: i64) -> Result<i64, i64> {
    if x < 0 {
        return Result::Err(4)
    }
    return Result::Ok(x)
}
fn chain(x: i64) -> Result<i64, i64> {
    let r = Rc::new(10)
    let a = probe(x)?
    return Result::Ok(a + Rc::get(r))
}
fn main() -> i64 {
    return match chain(0 - 1) { Result::Ok(v) => v, Result::Err(e) => e }
}
"#,
        Oracle::Balanced(4),
    ),
    (
        "SI-R03",
        r#"
struct Node { next: Rc<i64> }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let n: Node = Node { next: a }
    let m: Node = n
    return Rc::get(m.next)
}
"#,
        Oracle::Balanced(7),
    ),
    (
        "SI-R04",
        r#"
struct Node { next: Rc<i64> }
fn make(v: i64) -> Node {
    return Node { next: Rc::new(v) }
}
fn main() -> i64 {
    let n = make(6)
    return Rc::get(n.next)
}
"#,
        Oracle::Balanced(6),
    ),
    (
        "SI-R05",
        r#"
fn make() -> Rc<i64> {
    return Rc::new(42)
}
fn main() -> i64 {
    discard make()
    return 0
}
"#,
        Oracle::AllocsAt("-O0", OptimizationLevel::None, 0, 1),
    ),
    (
        "SI-R06",
        r#"
fn main() -> i64 {
    let mut i = 0
    let mut acc = 0
    while i < 100 {
        let r = Rc::new(i)
        acc = acc + Rc::get(r)
        i = i + 1
    }
    return acc % 7
}
"#,
        Oracle::Balanced(1),
    ),
];

const GROUP_E: &[(&str, &str, Oracle)] = &[
    (
        "SI-E01",
        r#"
enum Result<T, E> { Ok(T), Err(E) }
fn inner(x: i64) -> Result<i64, i64> {
    if x < 0 {
        return Result::Err(4)
    }
    return Result::Ok(x + 1)
}
fn outer(x: i64) -> Result<i64, i64> {
    let a = inner(x)?
    println(7)
    let b = inner(a)?
    return Result::Ok(a + b)
}
fn main() -> i64 {
    return match outer(1) { Result::Ok(v) => v, Result::Err(e) => e }
}
"#,
        Oracle::ExitOut(5, "7\n"),
    ),
    (
        // the err path short-circuits, so the marker must not print
        "SI-E02",
        r#"
enum Result<T, E> { Ok(T), Err(E) }
fn inner(x: i64) -> Result<i64, i64> {
    if x < 0 {
        return Result::Err(4)
    }
    return Result::Ok(x + 1)
}
fn outer(x: i64) -> Result<i64, i64> {
    let a = inner(x)?
    println(7)
    let b = inner(a)?
    return Result::Ok(a + b)
}
fn main() -> i64 {
    return match outer(0 - 1) { Result::Ok(v) => v, Result::Err(e) => e }
}
"#,
        Oracle::ExitOut(4, ""),
    ),
    (
        "SI-E03",
        r#"
enum Result<T, E> { Ok(T), Err(E) }
enum Fault { X }
fn bad() -> Result<i64, Fault> { return Result::Err(Fault::X) }
fn good() -> Result<i64, Fault> { return Result::Ok(7) }
fn main() -> i64 {
    let a = bad() catch |e| 100
    let b = good() catch |e| 100
    return a + b
}
"#,
        Oracle::Exit(107),
    ),
    (
        "SI-E04",
        r#"
fn noise() -> i64 {
    println(7)
    return 1
}
fn main() -> i64 {
    discard noise()
    return 0
}
"#,
        Oracle::ExitOut(0, "7\n"),
    ),
];

#[test]
fn group_re_rc_liveness_and_error_handling() {
    let h = Harness::new();
    run_rows(&h, GROUP_R);
    run_rows(&h, GROUP_E);
}

// divergence class an absolute oracle at every level closes.

const GROUP_A: &[(&str, &str, Oracle)] = &[
    (
        "SI-A01",
        r#"
fn main() -> i64 {
    let a = 0 - 9223372036854775807 - 1
    let b = 0 - 1
    return a / b
}
"#,
        Oracle::Exit(134),
    ),
    (
        "SI-A02",
        r#"
fn main() -> i64 {
    let a = 0 - 9223372036854775807 - 1
    let b = 0 - 1
    return a % b
}
"#,
        Oracle::Exit(134),
    ),
    (
        "SI-A03",
        r#"
fn main() -> i64 {
    let r: Rc<i64> = Rc::null()
    return Rc::get(r)
}
"#,
        Oracle::Exit(134),
    ),
    (
        "SI-A04",
        r#"
fn main() -> i64 {
    let a = 12
    let b = 0
    return a / b
}
"#,
        Oracle::Exit(134),
    ),
    (
        "SI-A05",
        r#"
fn main() -> i64 {
    let a = 0 - 8
    let b = 0 - 1
    return 7 / 2 + a / b - 8
}
"#,
        Oracle::Exit(3),
    ),
    (
        "SI-A06",
        r#"
fn main() -> i64 {
    let r = Rc::new(42)
    return Rc::get(r)
}
"#,
        Oracle::Balanced(42),
    ),
];

#[test]
fn group_a_trap_determinism() {
    let h = Harness::new();
    run_rows(&h, GROUP_A);
}

struct XRow {
    id: &'static str,
    code: &'static str,
    rejected: &'static str,
    // none where the rejected form has no kept counterpart
    twin: Option<(&'static str, i32)>,
}

const GROUP_X: &[XRow] = &[
    XRow {
        id: "SI-X01",
        code: "E0426",
        rejected: r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let s = v[0..2]
    s[0] = 99
    return v[0]
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let s = v[0..2]
    return s[0]
}
"#,
            1,
        )),
    },
    XRow {
        id: "SI-X02",
        code: "E0414",
        rejected: r#"
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let mut total = 0
    for x in v {
        total = total + x
    }
    return total
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let a = [1, 2, 3]
    let mut total = 0
    for x in a {
        total = total + x
    }
    return total
}
"#,
            6,
        )),
    },
    XRow {
        id: "SI-X03",
        code: "E0415",
        rejected: r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let r = &mut v[0]
    *r = 9
    return v[0]
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let mut x = 3
    let r = &mut x
    *r = 9
    return x
}
"#,
            9,
        )),
    },
    XRow {
        // stage 5 widened e0415 to a field spine. this program used to compile and silently drop the
        id: "SI-X15",
        code: "E0415",
        rejected: r#"
struct P { x: i64, y: i64 }
fn main() -> i64 {
    let mut p = P{x: 1, y: 2}
    let r = &mut p.x
    *r = 9
    return p.x
}
"#,
        twin: Some((
            r#"
struct P { x: i64, y: i64 }
fn main() -> i64 {
    let p = P{x: 7, y: 2}
    let r = &p.x
    return *r
}
"#,
            7,
        )),
    },
    XRow {
        id: "SI-X05",
        code: "E0417",
        rejected: r#"
fn main() -> i64 {
    let x = 1
    let a = &mut x
    *a = 2
    return x
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let mut x = 1
    let a = &mut x
    *a = 2
    return x
}
"#,
            2,
        )),
    },
    XRow {
        id: "SI-X06",
        code: "E0418",
        rejected: r#"
fn pick<A, B>(a: A, b: B) -> A {
    return a
}
fn other() -> i64 {
    fn pick<A, B>(a: A, b: B) -> B {
        return b
    }
    return pick(1, 2)
}
fn main() -> i64 {
    return pick(7, 8)
}
"#,
        twin: Some((
            r#"
fn pick<A, B>(a: A, b: B) -> A {
    return a
}
fn other() -> i64 {
    fn choose<A, B>(a: A, b: B) -> B {
        return b
    }
    return choose(1, 2)
}
fn main() -> i64 {
    return pick(7, 8)
}
"#,
            7,
        )),
    },
    XRow {
        id: "SI-X07",
        code: "E0419",
        rejected: r#"
enum Vec { Empty, One(i64) }
fn main() -> i64 { return 0 }
"#,
        twin: Some((
            r#"
enum MyList { Empty, One(i64) }
struct RefCount { count: i64 }
fn main() -> i64 { return 0 }
"#,
            0,
        )),
    },
    XRow {
        id: "SI-X08",
        code: "E0420",
        rejected: r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = (b)
    return 0
}
"#,
        twin: Some((
            r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = b
    return 0
}
"#,
            0,
        )),
    },
    XRow {
        id: "SI-X09",
        code: "E0412",
        rejected: r#"
fn main() -> i64 {
    let mut inner = Vec::new()
    Vec::push(inner, 1)
    let mut vv = Vec::new()
    Vec::push(vv, inner)
    return 0
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let mut flat = Vec::new()
    Vec::push(flat, 1)
    Vec::push(flat, 2)
    return flat[0] + flat[1]
}
"#,
            3,
        )),
    },
    XRow {
        id: "SI-X10",
        code: "E0412",
        rejected: r#"
fn main() -> i64 {
    let mut inner = Vec::new()
    Vec::push(inner, 1)
    let arr = [inner]
    return 0
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let arr = [1, 2, 3]
    return arr[0] + arr[2]
}
"#,
            4,
        )),
    },
    XRow {
        id: "SI-X11",
        code: "E0412",
        rejected: r#"
fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 {
    let v = vec[1, 2]
    return keep(v)
}
"#,
        twin: Some((
            r#"
fn keep<T>(x: T) -> i64 { let y = x; return 0 }
fn main() -> i64 {
    return keep(5) + 4
}
"#,
            4,
        )),
    },
    XRow {
        id: "SI-X12",
        code: "E0412",
        rejected: r#"
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let mut w = (v)
    w[0] = 9
    return v[0]
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let mut w = v
    w[0] = 9
    return v[0]
}
"#,
            1,
        )),
    },
    XRow {
        id: "SI-X13",
        code: "E0412",
        rejected: r#"
fn pick(a: Vec<i64>, c: i64) -> Vec<i64> {
    return if c == 1 { a } else { a }
}
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let w = pick(v, 1)
    return w[0]
}
"#,
        twin: Some((
            r#"
fn pick(a: Vec<i64>, c: i64) -> Vec<i64> {
    if c == 1 {
        return a
    }
    return a
}
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let w = pick(v, 1)
    return w[0]
}
"#,
            1,
        )),
    },
    XRow {
        id: "SI-X14",
        code: "E0304",
        rejected: r#"
fn main() -> i64 {
    let mut s = "hello"
    s[0] = "z"
    return 0
}
"#,
        twin: None,
    },
];

#[test]
fn group_x_deferred_surface_fails_closed() {
    let h = Harness::new();
    for row in GROUP_X {
        for (name, opt) in REJECT_LEVELS {
            let rendered = h.reject(row.id, name, row.rejected, *opt);
            assert!(
                rendered.contains(&format!("[{}]", row.code)),
                "{} at {name} MUST be rejected with {}, got:\n{rendered}",
                row.id,
                row.code
            );
        }
        if let Some((twin, exit)) = row.twin {
            let twin_id = format!("{}-twin", row.id);
            for (name, opt) in REJECT_LEVELS {
                h.accepts(&twin_id, name, twin, *opt);
            }
            check(&h, &twin_id, twin, exit, None, Memory::Ignore);
        }
    }
}

// p5, p10/p11, and the `rc::get` root the inventory has no row for) and every writer whose target
// pre-state, so a row cannot pass on a stale-but-equal read.

const GROUP_PA: &[(&str, &str, Oracle)] = &[
    (
        "SI-PA01",
        r#"
struct Cell { f: i64, g: i64 }
fn poke(pr: &mut Cell) -> i64 {
    (*pr).f = 101
    return 0
}
fn main() -> i64 {
    let mut c: Cell = Cell { f: 6031, g: 0 }
    let q = poke(&mut c)
    println(c.f)
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        "SI-PA02",
        r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [67, 2, 3]
    let r = &mut a
    (*r)[0] = 101
    println(a[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        // the m3 root: `rc::get(x)` denotes storage and place_of has no arm for it.
        "SI-PA03",
        r#"
struct Cell { f: i64, g: i64 }
fn main() -> i64 {
    let rc: Rc<Cell> = Rc::new(Cell { f: 6011, g: 0 })
    Rc::get(rc).f = 101
    println(Rc::get(rc).f)
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        "SI-PA04",
        r#"
struct Ar { a: [i64; 3] }
fn main() -> i64 {
    let mut ar: Ar = Ar { a: [1903, 2, 3] }
    let sv = ar.a[..]
    sv[0] = 101
    println(ar.a[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        // the push target is a place. sigabrt 134 `index out of bounds` at the sentinel,
        // because the open-coded unwrap named the pointer local as if it were the vec
        "SI-PA05",
        r#"
fn addone(r: &mut Vec<i64>) -> i64 {
    Vec::push((*r), 101)
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919]
    let q = addone(&mut v)
    println(v[1])
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        // must keep working"), and place::global must not change that. the `&mut g` witness that
        "SI-PA06",
        r#"
struct S { f: i64 }
let mut ga: [i64; 3] = [1901, 2, 3]
let mut gs: S = S { f: 1901 }
fn main() -> i64 {
    ga[0] = 101
    gs.f = 101
    println(ga[0])
    println(gs.f)
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n101\n"),
    ),
    (
        "SI-PA07",
        r#"
struct In { z: [i64; 3] }
struct Out { i: In }
fn poke(p: &mut Out) -> i64 {
    (*p).i.z[0] = 101
    return 0
}
fn main() -> i64 {
    let mut o: Out = Out { i: In { z: [7, 8, 9] } }
    let q = poke(&mut o)
    println(o.i.z[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        "SI-PA08",
        r#"
struct L4 { v: i64 }
struct L3 { d: L4 }
struct L2 { c: L3 }
struct L1 { b: L2 }
fn poke(p: &mut L1) -> i64 {
    (*p).b.c.d.v = 101
    return 0
}
fn main() -> i64 {
    let mut x: L1 = L1 { b: L2 { c: L3 { d: L4 { v: 7 } } } }
    let q = poke(&mut x)
    println(x.b.c.d.v)
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        "SI-PA09",
        r#"
struct Inner { z: [i64; 2] }
struct Outer { rows: [Inner; 2] }
fn poke(p: &mut Outer) -> i64 {
    (*p).rows[1].z[1] = 101
    return 0
}
fn main() -> i64 {
    let mut o: Outer = Outer { rows: [Inner { z: [1, 2] }, Inner { z: [3, 4] }] }
    let q = poke(&mut o)
    println(o.rows[1].z[1])
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        "SI-PA10",
        r#"
fn poke(r: &mut i64) -> i64 {
    (*r) = 101
    return 0
}
fn main() -> i64 {
    let mut x: i64 = 7
    let q = poke(&mut x)
    println(x)
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        "SI-PA11",
        r#"
fn bump(pr: &mut i64) -> i64 {
    (*pr)++
    return 0
}
fn main() -> i64 {
    let mut x: i64 = 100
    let q = bump(&mut x)
    println(x)
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        // w26: compound assignment through a projected pointer target
        "SI-PA12",
        r#"
struct Cell { f: i64, g: i64 }
fn bump(pr: &mut Cell) -> i64 {
    (*pr).f += 94
    return 0
}
fn main() -> i64 {
    let mut c: Cell = Cell { f: 7, g: 0 }
    let q = bump(&mut c)
    println(c.f)
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    // ---- p10/p11: captures are pointers into the env --
    (
        // a capture is a pointer into the env, so a projected write into a captured
        // the row exists so deleting the write-back cannot silently break it
        "SI-PA13",
        r#"
struct Cell { f: i64, g: i64 }
fn main() -> i64 {
    let mut c: Cell = Cell { f: 1, g: 0 }
    let f = fn () -> i64 {
        c.f = c.f + 1
        println(c.f)
        return c.f
    }
    let a = f()
    let b = f()
    println(c.f)
    return 0
}
"#,
        Oracle::ExitOut(0, "2\n3\n1\n"),
    ),
    (
        "SI-PA13b",
        r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [1, 2, 3]
    let f = fn () -> i64 {
        a[0] = a[0] + 1
        println(a[0])
        return a[0]
    }
    let x = f()
    let y = f()
    println(a[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "2\n3\n1\n"),
    ),
    (
        "SI-PA14",
        r#"
fn main() -> i64 {
    let mut c: i64 = 1
    let f = fn () -> i64 {
        c = c + 1
        println(c)
        return c
    }
    let a = f()
    let b = f()
    println(c)
    return 0
}
"#,
        Oracle::ExitOut(0, "2\n3\n1\n"),
    ),
    (
        // the 389-fixture corpus has zero `&mut` and vec_value_semantics_tests has no
        "SI-PA15",
        r#"
fn poke(r: &mut Vec<i64>) -> i64 {
    (*r)[0] = 101
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    let w: Vec<i64> = v
    let q = poke(&mut v)
    println(v[0])
    println(w[0])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "101\n7919\n", 2, 2),
    ),
    (
        // still be freed exactly once. this is what catches emit_vec_detach's pointer form
        "SI-PA16",
        r#"
fn poke(r: &mut Vec<i64>) -> i64 {
    (*r)[0] = 101
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    let q = poke(&mut v)
    return v[0] - 101
}
"#,
        Oracle::Balanced(0),
    ),
    (
        // a write through a pointer into rc payload storage, rc[n/n]
        "SI-PA17",
        r#"
struct Node { val: i64, next: Rc<Node> }
fn bump(p: &mut Node) -> i64 {
    (*p).val = 101
    return 0
}
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let p = &mut Rc::get(a)
    let q = bump(p)
    return Rc::get(a).val - 101
}
"#,
        Oracle::Balanced(0),
    ),
    (
        "SI-PA18",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7, 8, 9]
    let f = fn () -> i64 {
        return v[0]
    }
    return f() - 7
}
"#,
        Oracle::Leaks(
            0,
            "the closure env is a deliberate owner of the captured Vec buffer",
        ),
    ),
    (
        "SI-PA19",
        r#"
struct Cell { f: i64, g: i64 }
fn main() -> i64 {
    let n: Rc<Cell> = Rc::null()
    n.f = 101
    println(7919)
    return 0
}
"#,
        Oracle::ExitOutErr(134, "", "null pointer dereference"),
    ),
    (
        "SI-PA20",
        r#"
struct Cell { f: i64, g: i64 }
fn main() -> i64 {
    let n: Rc<Cell> = Rc::null()
    Rc::get(n).f = 101
    println(7919)
    return 0
}
"#,
        Oracle::ExitOutErr(134, "", "null pointer dereference"),
    ),
    (
        "SI-PA21",
        r#"
struct Cell { f: i64, g: i64 }
fn drain(pr: &mut Cell) -> i64 {
    while (*pr).f > 0 {
        (*pr).f = (*pr).f - 1
    }
    return 0
}
fn main() -> i64 {
    let mut c: Cell = Cell { f: 5, g: 0 }
    let q = drain(&mut c)
    println(c.f)
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "0\n"),
    ),
    (
        // the c53 self-call twin: the counter lives behind the pointer across a recursive call
        "SI-PA22",
        r#"
struct Cell { f: i64, g: i64 }
fn drain(pr: &mut Cell) -> i64 {
    if (*pr).f > 0 {
        (*pr).f = (*pr).f - 1
        return drain(pr)
    }
    return 0
}
fn main() -> i64 {
    let mut c: Cell = Cell { f: 5, g: 0 }
    let q = drain(&mut c)
    println(c.f)
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "0\n"),
    ),
    (
        // the sentinel printed an aslr-varying address that differed per allocator and per run,
        "SI-PA23",
        r#"
fn main() -> i64 {
    let a: [i64; 4] = [1, 2, 3, 4]
    let s = a[0..4]
    let t = s[0..2]
    println(t[0])
    println(t[1])
    return 0
}
"#,
        Oracle::AllocAgree(0),
    ),
    (
        // v1: `&v[0]` on a vec compiles and reads right. it never reaches the detach dispatch,
        // so retiring e0415/e0413/e0422 must fail a test rather than open the hole silently
        "SI-PA24",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1973, 2, 3]
    let e = &v[0]
    println(*e)
    return 0
}
"#,
        Oracle::ExitOut(0, "1973\n"),
    ),
    (
        // diii: a triply-nested index spine through a pointer root, rooted at a local because
        "SI-PA09b",
        r#"
fn main() -> i64 {
    let mut a: [[[i64; 2]; 2]; 2] = [[[1, 2], [3, 4]], [[5, 6], [7, 8]]]
    let r = &mut a
    (*r)[1][1][1] = 101
    println(a[1][1][1])
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        // v2: the same through a pointer root
        "SI-PA25",
        r#"
fn peek(r: &mut Vec<i64>) -> i64 {
    let e = &(*r)[0]
    return *e
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    let a = peek(&mut v)
    println(a)
    return 0
}
"#,
        Oracle::ExitOut(0, "7919\n"),
    ),
    // a capture is a pointer, so every site that reads a capture's
    (
        // place against rvalue, so it compiled clean. pre-fix: an aslr-varying integer that
        "SI-PA26",
        r#"
fn main() -> i64 {
    let c: i64 = 7
    let outer = fn () -> i64 {
        let inner = fn () -> i64 {
            return c
        }
        return inner()
    }
    println(outer())
    return 0
}
"#,
        Oracle::ExitOut(0, "7\n"),
    ),
    (
        "SI-PA27",
        r#"
struct S { f: i64, g: i64 }
fn main() -> i64 {
    let s: S = S { f: 7, g: 0 }
    let outer = fn () -> i64 {
        let inner = fn () -> i64 {
            return s.f
        }
        return inner()
    }
    println(outer())
    return 0
}
"#,
        Oracle::ExitOut(0, "7\n"),
    ),
    (
        "SI-PA28",
        r#"
fn main() -> i64 {
    let a: [i64; 3] = [7, 8, 9]
    let outer = fn () -> i64 {
        let inner = fn () -> i64 {
            return a[0]
        }
        return inner()
    }
    println(outer())
    return 0
}
"#,
        Oracle::ExitOut(0, "7\n"),
    ),
    (
        // a 32-byte struct: the store into the inner env field was 8 bytes wide (a pointer)
        "SI-PA29",
        r#"
struct Big { a: i64, b: i64, c: i64, d: i64 }
fn main() -> i64 {
    let s: Big = Big { a: 1, b: 2, c: 3, d: 4 }
    let outer = fn () -> i64 {
        let inner = fn () -> i64 {
            return s.a + s.b + s.c + s.d
        }
        return inner()
    }
    println(outer())
    return 0
}
"#,
        Oracle::ExitOut(0, "10\n"),
    ),
    (
        // a captured vec: the inner env's {ptr,len,cap} header was overwritten with a pointer,
        // so the read went out of bounds. pre-fix: sigabrt 134 at every level, compile-clean
        "SI-PA30",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[7, 8, 9]
    let outer = fn () -> i64 {
        let inner = fn () -> i64 {
            return v[0]
        }
        return inner()
    }
    println(outer())
    return 0
}
"#,
        Oracle::ExitOut(0, "7\n"),
    ),
    (
        // miscompiled. check() walks -o0/-o2/-o3, so the row fails on the -o0 leg
        "SI-PA31",
        r#"
fn make_f() -> fn() -> fn() -> i64 {
    let a: i64 = 1
    return fn() -> fn() -> i64 {
        let b: i64 = 2
        return fn() -> i64 {
            return a + b
        }
    }
}
fn main() -> i64 {
    let f = make_f()
    let g = f()
    let h = g()
    println(h)
    return 0
}
"#,
        Oracle::ExitOut(0, "3\n"),
    ),
    (
        // a pointer, so the runtime decremented the closure env's own rc header. one iteration
        // only leaked (frees 1 -> 0), which balanced and namedleak both accept; four iterations
        "SI-PA32",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let f = fn () -> i64 {
        v = vec[9, 9, 9]
        return v[0]
    }
    for i in 0..4 {
        println(f())
    }
    println(v[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "9\n9\n9\n9\n1\n"),
    ),
    (
        "SI-PA33",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let f = fn () -> i64 {
        v = vec[9, 9, 9]
        return v[0]
    }
    let a = f()
    println(a)
    println(v[0])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "9\n1\n", 3, 1),
    ),
];

#[test]
fn group_pa_place_addressing() {
    let h = Harness::new();
    run_rows(&h, GROUP_PA);
}

// the fail-closed half: one row per code stage 1 adds, plus the two c52 legs and the origins
const GROUP_PA_X: &[XRow] = &[
    XRow {
        id: "SI-PAX01",
        code: "E0421",
        rejected: r#"
struct Cell { f: i64, g: i64 }
fn mkc() -> Cell {
    return Cell { f: 7, g: 0 }
}
fn main() -> i64 {
    mkc().f = 101
    return 0
}
"#,
        twin: Some((
            r#"
struct Cell { f: i64, g: i64 }
fn mkc() -> Cell {
    return Cell { f: 7, g: 0 }
}
fn main() -> i64 {
    let mut c: Cell = mkc()
    c.f = 101
    return c.f - 100
}
"#,
            1,
        )),
    },
    XRow {
        id: "SI-PAX02",
        code: "E0422",
        rejected: r#"
struct Cell { f: i64, g: i64 }
fn poke(r: &Cell) -> i64 {
    (*r).f = 101
    return 0
}
fn main() -> i64 {
    let mut s: Cell = Cell { f: 6031, g: 0 }
    let q = poke(&s)
    return s.f
}
"#,
        twin: Some((
            r#"
struct Cell { f: i64, g: i64 }
fn poke(r: &mut Cell) -> i64 {
    (*r).f = 1
    return 0
}
fn main() -> i64 {
    let mut s: Cell = Cell { f: 6031, g: 0 }
    let q = poke(&mut s)
    return s.f
}
"#,
            1,
        )),
    },
    XRow {
        id: "SI-PAX03",
        code: "E0422",
        rejected: r#"
fn poke(r: &i64) -> i64 {
    *r = 101
    return 0
}
fn main() -> i64 {
    let x: i64 = 7
    let q = poke(&x)
    return x
}
"#,
        twin: Some((
            r#"
fn poke(r: &mut i64) -> i64 {
    *r = 1
    return 0
}
fn main() -> i64 {
    let mut x: i64 = 7
    let q = poke(&mut x)
    return x
}
"#,
            1,
        )),
    },
    XRow {
        id: "SI-PAX04",
        code: "E0423",
        rejected: r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let f = fn () -> i64 {
        let e = &v[0]
        return *e
    }
    return f()
}
"#,
        twin: Some((
            r#"
fn apply(f: fn(&i64) -> i64, p: &i64) -> i64 {
    return f(p)
}
fn main() -> i64 {
    let n: i64 = 1
    let r = &n
    let g = fn (p: &i64) -> i64 {
        return *p
    }
    return apply(g, r)
}
"#,
            1,
        )),
    },
    XRow {
        id: "SI-PAX05",
        code: "E0424",
        rejected: r#"
let mut g: i64 = 1901
fn main() -> i64 {
    let r = &mut g
    *r = 101
    return g
}
"#,
        twin: Some((
            r#"
struct S { f: i64 }
let mut ga: [i64; 3] = [1901, 2, 3]
let mut gs: S = S { f: 1901 }
fn main() -> i64 {
    ga[0] = 1
    gs.f = 0
    return ga[0] + gs.f
}
"#,
            1,
        )),
    },
    XRow {
        id: "SI-PAX06",
        code: "E0721",
        // pass as if it borrowed the caller. stage 1 makes that address real
        rejected: r#"
struct Cell { f: i64, g: i64 }
fn leak(c: Cell) -> &i64 {
    return &c.f
}
fn main() -> i64 {
    let x: Cell = Cell { f: 7, g: 0 }
    let r = leak(x)
    return *r
}
"#,
        twin: Some((
            r#"
fn passthru(p: &i64) -> &i64 {
    return p
}
fn main() -> i64 {
    let x: i64 = 1
    let r = passthru(&x)
    return *r
}
"#,
            1,
        )),
    },
    XRow {
        id: "SI-PAX06b",
        code: "E0423",
        rejected: r#"
fn main() -> i64 {
    let mut c: i64 = 1
    let f = fn () -> i64 {
        let r = &mut c
        *r = c + 1
        return c
    }
    return f()
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let mut c: i64 = 0
    let f = fn () -> i64 {
        c = c + 1
        return c
    }
    return f()
}
"#,
            1,
        )),
    },
    XRow {
        id: "SI-PAX07",
        code: "E0711",
        // borrow checker, not by anything designed for it
        rejected: r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let e = &v[0]
    Vec::push(v, 4)
    Vec::push(v, 5)
    return *e
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX08",
        code: "E0423",
        rejected: r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let f = fn () -> i64 {
        let e = &v[0]
        return *e
    }
    return f()
}
"#,
        twin: None,
    },
];

#[test]
fn group_pa_fails_closed() {
    let h = Harness::new();
    for row in GROUP_PA_X {
        for (name, opt) in REJECT_LEVELS {
            let rendered = h.reject(row.id, name, row.rejected, *opt);
            assert!(
                rendered.contains(&format!("[{}]", row.code))
                    || rendered.contains(&format!("error[{}]", row.code)),
                "{} at {name} MUST be rejected with {}, got:\n{rendered}",
                row.id,
                row.code
            );
        }
        if let Some((twin, exit)) = row.twin {
            let twin_id = format!("{}-twin", row.id);
            for (name, opt) in REJECT_LEVELS {
                h.accepts(&twin_id, name, twin, *opt);
            }
            check(&h, &twin_id, twin, exit, None, Memory::Ignore);
        }
    }
}

const GROUP_S2: &[(&str, &str, Oracle)] = &[
    (
        // the formation bounds check: silent -o-divergent garbage at base, a named trap now
        "S2-B01",
        r#"
fn main() -> i64 {
    let a: [i64; 3] = [1, 2, 3]
    let s = a[0..10]
    println(s[0])
    return 0
}
"#,
        Oracle::ExitOutErr(134, "", "slice range out of bounds"),
    ),
    (
        // a runtime end bound, so the check cannot be folded away
        "S2-B03",
        r#"
fn main() -> i64 {
    let a: [i64; 3] = [1, 2, 3]
    let mut n = 3
    n = n + 7
    let s = a[0..n]
    println(s[0])
    return 0
}
"#,
        Oracle::ExitOutErr(134, "", "slice range out of bounds"),
    ),
    (
        "S2-B04",
        r#"
fn main() -> i64 {
    let a: [i64; 3] = [11, 22, 33]
    let s = a[0..3]
    return s[1]
}
"#,
        Oracle::ExitOut(22, ""),
    ),
    (
        "S2-B05",
        r#"
fn main() -> i64 {
    let a: [i64; 3] = [11, 22, 33]
    let s = a[0..0]
    return 7
}
"#,
        Oracle::ExitOut(7, ""),
    ),
    (
        "S2-B06",
        r#"
fn main() -> i64 {
    let a: [i64; 3] = [1, 2, 3]
    let s = a[0..10]
    return 7
}
"#,
        Oracle::ExitOutErr(134, "", "slice range out of bounds"),
    ),
    (
        "S2-B07",
        r#"
nogc fn f(a: &[i64], n: i64) -> i64 {
    let s = a[0..n]
    return 0
}
fn main() -> i64 {
    let arr: [i64; 3] = [1, 2, 3]
    return f(arr[..], 9)
}
"#,
        Oracle::ExitOutErr(134, "", "slice range out of bounds"),
    ),
    (
        "S2-B08",
        r#"
fn n() -> i64 {
    return 0 - 1
}
fn main() -> i64 {
    let a: [i64; 3] = [1, 2, 3]
    let s = a[0..n()]
    return 7
}
"#,
        Oracle::ExitOutErr(134, "", "slice range out of bounds"),
    ),
    (
        // the origins companion must not over-reject a re-slice of a reference-typed parameter
        "S2-L08",
        r#"
fn f(a: &[i64]) -> &[i64] {
    let s = a[0..4]
    return s[0..2]
}
fn main() -> i64 {
    let arr: [i64; 4] = [1, 2, 3, 4]
    let r = f(arr[..])
    return r[1]
}
"#,
        Oracle::ExitOut(2, ""),
    ),
    (
        "S2-L10a",
        r#"
fn main() -> i64 {
    let a: [i64; 3] = [1, 2, 3]
    let s = a[0..2]
    return s[0] + a[1]
}
"#,
        Oracle::ExitOut(3, ""),
    ),
    (
        "S2-L10b",
        r#"
fn main() -> i64 {
    let a: [i64; 3] = [1, 2, 3]
    let s = a[0..2]
    let t = a[0..3]
    return s[0] + t[2]
}
"#,
        Oracle::ExitOut(4, ""),
    ),
    (
        "S2-L10c",
        r#"
fn main() -> i64 {
    let a: [i64; 4] = [1, 2, 3, 4]
    let s = a[0..4]
    let t = s[0..2]
    return t[1]
}
"#,
        Oracle::ExitOut(2, ""),
    ),
    (
        // a slice of a vec addresses the buffer, which is the capability the stage delivers
        "S2-A01",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..2]
    println(s[1])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "22\n", 1, 1),
    ),
    (
        "S2-A02",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..]
    println(s[1])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "22\n", 1, 1),
    ),
    (
        "S2-A03",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    let s = v[..]
    println(s[1])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "22\n", 1, 1),
    ),
    (
        "S2-A04",
        r#"
fn f(v: Vec<i64>) -> i64 {
    let s = v[0..2]
    return s[1]
}
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    return f(v)
}
"#,
        Oracle::ExitOut(22, ""),
    ),
    (
        "S2-A05",
        r#"
fn f(r: &Vec<i64>) -> i64 {
    let s = (*r)[0..2]
    return s[1]
}
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    return f(&v)
}
"#,
        Oracle::ExitOut(22, ""),
    ),
    (
        "S2-A06",
        r#"
fn main() -> i64 {
    let v = vec[11, 22, 33]
    println(v[..][1])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "22\n", 1, 1),
    ),
    (
        // seven elements, so the length and the element cannot coincide
        "S2-A07",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33, 44, 55, 66, 77]
    let s = v[..]
    println(s[1])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "22\n", 1, 1),
    ),
    (
        "S2-A08",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..0]
    println(7)
    return 0
}
"#,
        Oracle::ExitOutStats(0, "7\n", 1, 1),
    ),
    (
        "S2-A09",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[7]
    let s = v[0..1]
    println(s[0])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "7\n", 1, 1),
    ),
    (
        "S2-A10",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..3]
    let t = s[0..2]
    println(t[1])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "22\n", 1, 1),
    ),
    (
        // the read shape e0426's interprocedural half must not cost
        "S2-A11",
        r#"
fn sum(s: &[i64]) -> i64 {
    return s[0] + s[1]
}
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    return sum(v[..])
}
"#,
        Oracle::ExitOut(33, ""),
    ),
    (
        "S2-A12",
        r#"
fn main() -> i64 {
    let a: [i64; 3] = [11, 22, 33]
    let s = a[0..3]
    let t = s[..]
    return t[1]
}
"#,
        Oracle::ExitOut(22, ""),
    ),
    (
        "S2-M05",
        r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [1, 2, 3]
    let s = a[0..2]
    s[0] = 99
    return a[0]
}
"#,
        Oracle::ExitOut(99, ""),
    ),
    (
        "S2-M06",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[1, 2, 3]
    let s = v[0..2]
    println(s[0])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "1\n", 1, 1),
    ),
    (
        "S2-M07",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let s = v[0..2]
    println(s[0])
    v[0] = 99
    println(v[0])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "1\n99\n", 1, 1),
    ),
    (
        // the base is born and buried inside the lambda, so nothing it borrows can outlive it
        "S2-F12",
        r#"
fn main() -> i64 {
    let f = fn () -> i64 {
        let a: [i64; 3] = [11, 22, 33]
        let s = a[0..2]
        return s[1]
    }
    return f()
}
"#,
        Oracle::ExitOut(22, ""),
    ),
    (
        "S2-M15",
        r#"
fn poke(r: &mut Vec<i64>) -> i64 {
    (*r)[0] = 99
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let w: Vec<i64> = v
    let z = poke(&mut v)
    println(w[0])
    println(v[0])
    return z
}
"#,
        Oracle::ExitOutStats(0, "11\n99\n", 2, 2),
    ),
    (
        "S2-M16",
        r#"
struct Ar { a: [i64; 3] }
fn poke(r: &Ar) -> i64 {
    let s = (*r).a[0..3]
    s[0] = 99
    return 0
}
fn main() -> i64 {
    let mut b = Ar { a: [11, 22, 33] }
    let z = poke(&b)
    println(b.a[0])
    return z
}
"#,
        Oracle::ExitOutStats(0, "99\n", 0, 0),
    ),
    (
        "S2-X02",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let w: Vec<i64> = v
    v[0] = 99
    let s = w[..]
    println(v[0])
    println(s[0])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "99\n11\n", 2, 2),
    ),
    (
        "S2-X04",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..2]
    let t = v[0..3]
    println(s[0] + t[2])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "44\n", 1, 1),
    ),
    (
        "S2-X03",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..2]
    println(s[1])
    Vec::push(v, 44)
    println(v[3])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "22\n44\n", 1, 1),
    ),
    (
        // by-value capture holds this: the env's share detaches on push, and frees=1 is the leak
        "S2-F11",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..2]
    let f = fn () -> i64 {
        Vec::push(v, 44)
        Vec::push(v, 45)
        Vec::push(v, 46)
        return 0
    }
    let q = f()
    println(s[1])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "22\n", 3, 1),
    ),
    (
        "S2-T01",
        r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [3, 0, 0]
    let s = a[0..3]
    let mut guard = 0
    while s[0] > 0 {
        s[0] = s[0] - 1
        guard = guard + 1
    }
    println(guard)
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "3\n"),
    ),
    (
        "S2-T03",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[0]
    let mut i = 1
    while i < 100 {
        Vec::push(v, i)
        i = i + 1
    }
    let s = v[..]
    println(s[99])
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "99\n"),
    ),
];

#[test]
fn group_s2_slice_of_vec() {
    let h = Harness::new();
    run_rows(&h, GROUP_S2);
}

const GROUP_S2_X: &[XRow] = &[
    XRow {
        id: "S2-F01",
        code: "E0425",
        rejected: r#"
fn main() -> i64 {
    let a: [i64; 4] = [1, 2, 3, 4]
    let s = a[1..3]
    return s[0]
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let a: [i64; 4] = [1, 2, 3, 4]
    let s = a[0..3]
    return s[1]
}
"#,
            2,
        )),
    },
    XRow {
        id: "S2-L01",
        code: "E0711",
        rejected: r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [1, 2, 3]
    let s = a[0..3]
    a[1] = 99
    return s[1]
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [1, 2, 3]
    let s = a[0..3]
    let x = s[1]
    a[1] = 99
    return x + a[1] - 97
}
"#,
            4,
        )),
    },
    XRow {
        id: "S2-L03",
        code: "E0722",
        rejected: r#"
fn main() -> i64 {
    let base: [i64; 3] = [0, 0, 0]
    let mut s = base[..]
    if true {
        let a: [i64; 3] = [1, 2, 3]
        s = a[..]
    }
    return s[1]
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let base: [i64; 3] = [0, 0, 0]
    let a: [i64; 3] = [1, 2, 3]
    let mut s = base[..]
    if true {
        s = a[..]
    }
    return s[1]
}
"#,
            2,
        )),
    },
    XRow {
        id: "S2-L09",
        code: "E0721",
        rejected: r#"
fn mk() -> &[i64] {
    let a: [i64; 3] = [1, 2, 3]
    return a[..]
}
fn main() -> i64 {
    let s = mk()
    return s[1]
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-L11",
        code: "E0721",
        rejected: r#"
fn id(s: &[i64]) -> &[i64] {
    return s
}
fn h() -> &[i64] {
    let a: [i64; 3] = [11, 22, 33]
    let s = a[0..3]
    let t = id(s)
    return t[0..2]
}
fn main() -> i64 {
    let u = h()
    return u[1]
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-L02",
        code: "E0711",
        rejected: r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..3]
    v[1] = 99
    return s[1]
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-L04",
        code: "E0711",
        rejected: r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..3]
    Vec::push(v, 44)
    return s[0]
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-L05",
        code: "E0711",
        rejected: r#"
fn f(r: &mut Vec<i64>) -> i64 {
    let s = (*r)[0..3]
    Vec::push((*r), 44)
    return s[0]
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    return f(&mut v)
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-L06",
        code: "E0711",
        rejected: r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..3]
    v = vec[9, 9, 9]
    return s[0]
}
"#,
        twin: None,
    },
    XRow {
        // without the borrow-of-a-reference edge s dies here and t[0] reads the freed buffer
        id: "S2-L07",
        code: "E0711",
        rejected: r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..3]
    let t = s[0..2]
    Vec::push(v, 44)
    return t[0]
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-L03v",
        code: "E0722",
        rejected: r#"
fn main() -> i64 {
    let base: Vec<i64> = vec[0, 0, 0]
    let mut s = base[..]
    if true {
        let a: Vec<i64> = vec[1, 2, 3]
        s = a[..]
    }
    return s[1]
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-M01",
        code: "E0426",
        rejected: r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let w: Vec<i64> = v
    let s = v[0..2]
    s[0] = 99
    println(v[0])
    println(w[0])
    return 0
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[1, 2, 3]
    let w: Vec<i64> = v
    let s = v[0..2]
    return s[0] + w[1]
}
"#,
            3,
        )),
    },
    XRow {
        id: "S2-M02",
        code: "E0426",
        rejected: r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let s = v[0..2]
    let t = s
    t[0] = 99
    return v[0]
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[1, 2, 3]
    let s = v[0..2]
    let t = s
    return t[0]
}
"#,
            1,
        )),
    },
    XRow {
        id: "S2-M03",
        code: "E0426",
        rejected: r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let s = v[0..3]
    let u = s[0..2]
    u[0] = 99
    return v[0]
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[1, 2, 3]
    let s = v[0..3]
    let u = s[0..2]
    return u[0]
}
"#,
            1,
        )),
    },
    XRow {
        id: "S2-M04",
        code: "E0426",
        rejected: r#"
fn poke(s: &[i64]) -> i64 {
    s[0] = 9
    return 0
}
fn main() -> i64 {
    let v: Vec<i64> = vec[1, 2, 3]
    let q = poke(v[..])
    return v[0]
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-M09",
        code: "E0426",
        rejected: r#"
fn d(s: &[i64]) -> i64 {
    let t = s[0..1]
    t[0] = 9
    return 0
}
fn main() -> i64 {
    let v: Vec<i64> = vec[1, 2, 3]
    let z = d(v[0..3])
    return v[0] + z
}
"#,
        twin: Some((
            r#"
fn d(s: &[i64]) -> i64 {
    let t = s[0..1]
    t[0] = 9
    return 0
}
fn main() -> i64 {
    let mut a: [i64; 3] = [1, 2, 3]
    let z = d(a[0..3])
    return a[0] + z
}
"#,
            9,
        )),
    },
    XRow {
        id: "S2-M10",
        code: "E0426",
        rejected: r#"
fn poke(r: &Vec<i64>) -> i64 {
    let s = (*r)[0..3]
    s[0] = 99
    return 0
}
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    let w: Vec<i64> = v
    let z = poke(&v)
    println(w[0])
    return z
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-M11",
        code: "E0426",
        rejected: r#"
fn poke(r: &mut Vec<i64>) -> i64 {
    let s = (*r)[0..3]
    s[0] = 99
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let w: Vec<i64> = v
    let z = poke(&mut v)
    println(w[0])
    println(v[0])
    return z
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-M12",
        code: "E0426",
        rejected: r#"
fn poke(r: &mut Vec<i64>) -> i64 {
    let s = (*r)[..]
    s[0] = 99
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let w: Vec<i64> = v
    let z = poke(&mut v)
    println(w[0])
    return z
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-M13",
        code: "E0426",
        rejected: r#"
fn wr(s: &[i64]) -> i64 {
    s[0] = 99
    return 0
}
fn poke(r: &mut Vec<i64>) -> i64 {
    let s = (*r)[0..3]
    return wr(s)
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let w: Vec<i64> = v
    let z = poke(&mut v)
    println(w[0])
    return z
}
"#,
        twin: None,
    },
    XRow {
        // a re-slice inside the callee, so the write's dest is two borrows from the referent
        id: "S2-M14",
        code: "E0426",
        rejected: r#"
fn poke(r: &mut Vec<i64>) -> i64 {
    let s = (*r)[0..3]
    let t = s[0..2]
    t[0] = 99
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let w: Vec<i64> = v
    let z = poke(&mut v)
    println(w[0])
    return z
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-F08",
        code: "E0423",
        rejected: r#"
fn main() -> i64 {
    let a: [i64; 3] = [11, 22, 33]
    let f = fn () -> i64 {
        let s = a[0..2]
        return s[1]
    }
    return f()
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-F07",
        code: "E0423",
        rejected: r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    let f = fn () -> i64 {
        let s = v[0..2]
        return s[1]
    }
    return f()
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-F14",
        code: "E0423",
        rejected: r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    let w: Vec<i64> = v
    let f = fn () -> i64 {
        let u = v
        let s = u[0..3]
        s[0] = 99
        return 0
    }
    let z = f()
    println(w[0])
    return z
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-F15",
        code: "E0423",
        rejected: r#"
fn main() -> i64 {
    let f = fn () -> i64 {
        let v: Vec<i64> = vec[11, 22, 33]
        let s = v[0..3]
        return s[1]
    }
    return f()
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-F09",
        code: "E0423",
        rejected: r#"
fn apply(g: fn(&[i64]) -> &[i64], s: &[i64]) -> i64 {
    let r = g(s)
    return r[0]
}
fn main() -> i64 {
    let a: [i64; 3] = [11, 22, 33]
    let f = fn (s: &[i64]) -> &[i64] {
        return s
    }
    return apply(f, a[..])
}
"#,
        twin: None,
    },
    XRow {
        // the widened gate has not swallowed it
        id: "S2-F10",
        code: "E0725",
        rejected: r#"
fn main() -> i64 {
    let a: [i64; 3] = [11, 22, 33]
    let s = a[0..2]
    let f = fn () -> i64 {
        return s[1]
    }
    return f()
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-C07",
        code: "E0711",
        rejected: r#"
fn f(r: &Vec<i64>) -> &[i64] {
    return (*r)[0..2]
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[11, 22, 33]
    let s = f(&v)
    Vec::push(v, 44)
    return s[0]
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-C07b",
        code: "E0721",
        rejected: r#"
fn f(v: Vec<i64>) -> &[i64] {
    return v[0..2]
}
fn main() -> i64 {
    let v: Vec<i64> = vec[1, 2, 3]
    let s = f(v)
    return s[0]
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-C08",
        code: "E0714",
        rejected: r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[1, 2, 3]
    let s = v[0..2]
    let arr = [s, s]
    return 0
}
"#,
        twin: None,
    },
    XRow {
        id: "S2-F05",
        code: "E0901",
        rejected: r#"
let gv: Vec<i64> = vec[1, 2, 3]
fn main() -> i64 {
    let s = gv[0..2]
    return s[0]
}
"#,
        twin: None,
    },
];

#[test]
fn s2_e0413_is_retired_and_its_heirs_are_registered() {
    use aelys_common::diagnostic::registry;
    assert!(
        registry::lookup("E0413").is_none(),
        "E0413 must be gone from the registry"
    );
    for code in ["E0425", "E0426"] {
        let info =
            registry::lookup(code).unwrap_or_else(|| panic!("{code} must have an --explain entry"));
        assert!(
            !info.explanation.trim().is_empty(),
            "{code}'s --explain entry must not be empty"
        );
    }
    for code in ["E0421", "E0422", "E0423", "E0424"] {
        assert!(
            registry::lookup(code).is_some(),
            "{code} must have an --explain entry"
        );
    }
}

#[test]
fn group_s2_fails_closed() {
    let h = Harness::new();
    for row in GROUP_S2_X {
        for (name, opt) in REJECT_LEVELS {
            let rendered = h.reject(row.id, name, row.rejected, *opt);
            assert!(
                rendered.contains(&format!("[{}]", row.code)),
                "{} at {name} MUST be rejected with {}, got:\n{rendered}",
                row.id,
                row.code
            );
        }
        if let Some((twin, exit)) = row.twin {
            let twin_id = format!("{}-twin", row.id);
            for (name, opt) in REJECT_LEVELS {
                h.accepts(&twin_id, name, twin, *opt);
            }
            check(&h, &twin_id, twin, exit, None, Memory::Ignore);
        }
    }
}

#[test]
fn s2_slice_form_refusal_is_opt_level_independent() {
    let h = Harness::new();
    let rows: &[(&str, &str)] = &[
        (
            "S2-F03",
            r#"
fn main() -> i64 {
    let t = "abcdef"
    let s = t[0..2]
    return 22
}
"#,
        ),
        (
            "S2-F03b",
            r#"
fn main() -> i64 {
    let t = "abcdef"
    let s = t[..]
    return 22
}
"#,
        ),
    ];
    for (id, src) in rows {
        for (name, opt) in LEVELS {
            let rendered = h.reject(id, name, src, *opt);
            assert!(
                rendered.contains("[E0425]"),
                "{id} at {name} MUST be rejected with E0425, got:\n{rendered}"
            );
        }
    }
}

#[test]
fn s2_generic_slice_refusal_is_opt_level_independent() {
    let h = Harness::new();
    let src = r#"
fn firstof<T>(x: T) -> i64 {
    let s = x[0..1]
    return 5
}
fn main() -> i64 {
    let a: [i64; 3] = [1, 2, 3]
    return firstof(a)
}
"#;
    for (name, opt) in LEVELS {
        let rendered = h.reject("S2-F13", name, src, *opt);
        assert!(
            rendered.contains("[E0901]"),
            "S2-F13 at {name} MUST be rejected, got:\n{rendered}"
        );
    }
}

#[cfg(feature = "asan-invariants")]
mod asan {
    use super::*;

    const ASAN_ROWS: &[(&str, &str, i32)] = &[
        (
            "SI-V01",
            r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let w = v
    v[0] = 9
    return w[0]
}
"#,
            1,
        ),
        (
            "SI-V04",
            r#"
fn poke(mut x: Vec<i64>) -> i64 {
    x[0] = 9
    return x[0]
}
fn main() -> i64 {
    let v = vec[1, 2, 3]
    let _ = poke(v)
    return v[0]
}
"#,
            1,
        ),
        (
            "SI-V10",
            r#"
fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    Vec::push(v, 2)
    Vec::push(v, 3)
    let w = v
    Vec::push(v, 4)
    Vec::push(v, 5)
    let junk = vec[77, 77, 77]
    return w[0]
}
"#,
            1,
        ),
        (
            "SI-R01",
            r#"
fn f(c: bool) -> i64 {
    let r = Rc::new(5)
    if c {
        return Rc::get(r)
    }
    return 0
}
fn main() -> i64 {
    return f(true)
}
"#,
            5,
        ),
        (
            "S2-A07",
            r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33, 44, 55, 66, 77]
    let s = v[..]
    return s[6] - 55
}
"#,
            22,
        ),
        (
            "S2-A10",
            r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    let s = v[0..3]
    let t = s[0..2]
    return t[1]
}
"#,
            22,
        ),
        (
            "S2-T03",
            r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[0]
    let mut i = 1
    while i < 100 {
        Vec::push(v, i)
        i = i + 1
    }
    let s = v[..]
    return s[99] - 77
}
"#,
            22,
        ),
    ];

    fn build_asan_archive(dir: &Path) -> Option<PathBuf> {
        let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
        let core_src = manifest
            .parent()
            .unwrap_or(manifest)
            .join("core")
            .join("src");
        let mut objects = Vec::new();
        for unit in [
            "aelys_core_common.c",
            "aelys_alloc_immix.c",
            "aelys_rc_real.c",
        ] {
            let src = core_src.join(unit);
            if !src.is_file() {
                eprintln!("core source {unit} missing; skipping the ASan tier");
                return None;
            }
            let obj = dir.join(unit).with_extension("o");
            match Command::new("clang")
                .args(["-fsanitize=address", "-g", "-c"])
                .arg(&src)
                .arg(format!("-I{}", core_src.display()))
                .arg("-o")
                .arg(&obj)
                .output()
            {
                Ok(out) if out.status.success() => objects.push(obj),
                Ok(out) => panic!(
                    "instrumented core compile of {unit} failed:\n{}",
                    String::from_utf8_lossy(&out.stderr)
                ),
                Err(_) => {
                    eprintln!("clang unavailable; skipping the ASan tier");
                    return None;
                }
            }
        }
        let archive = dir.join("libaelys-core-rc-asan.a");
        match Command::new("ar")
            .arg("rcs")
            .arg(&archive)
            .args(&objects)
            .output()
        {
            Ok(out) if out.status.success() => Some(archive),
            Ok(out) => panic!("ar failed:\n{}", String::from_utf8_lossy(&out.stderr)),
            Err(_) => {
                eprintln!("ar unavailable; skipping the ASan tier");
                None
            }
        }
    }

    #[test]
    fn asan_tier_value_rows_are_clean() {
        let h = Harness::new();
        let Some(_archive) = build_asan_archive(h.dir.path()) else {
            return;
        };

        for (id, src, expected) in ASAN_ROWS {
            let path = h.write(id, "asan", src);
            if compile_file_with_llvm_variant(
                &path,
                OptimizationLevel::None,
                false,
                RuntimeVariant::Rc,
            )
            .is_err()
            {
                eprintln!("{id}: toolchain unavailable, skipping");
                return;
            }
            let object = path.with_extension(if cfg!(windows) { "obj" } else { "o" });
            if !object.is_file() {
                eprintln!("{id}: object not produced, skipping");
                return;
            }
            let exe = h.dir.path().join(format!("{}_asan_exe", slug(id, "")));
            let link = Command::new("clang")
                .args(["-fsanitize=address", "-g"])
                .arg(&object)
                .arg(format!("-L{}", h.dir.path().display()))
                .arg("-laelys-core-rc-asan")
                .arg("-o")
                .arg(&exe)
                .output();
            let Ok(link) = link else {
                eprintln!("{id}: clang unavailable, skipping");
                return;
            };
            assert!(
                link.status.success(),
                "{id}: ASan link failed:\n{}",
                String::from_utf8_lossy(&link.stderr)
            );
            let run = Command::new(&exe)
                .env("AELYS_RC_STATS", "1")
                .env("ASAN_OPTIONS", "detect_leaks=1")
                .output()
                .expect("run the instrumented exe");
            let stderr = String::from_utf8_lossy(&run.stderr);
            assert_eq!(
                run.status.code(),
                Some(*expected),
                "{id} under ASan: the answer MUST be {expected}\nstderr:\n{stderr}"
            );
            assert!(
                !stderr.contains("AddressSanitizer"),
                "{id}: ASan reports a memory error\nstderr:\n{stderr}"
            );
        }
    }

    // the nested vec shape was a reproducible use-after-free returning 77; it now fails closed, so
    #[test]
    fn asan_tier_nested_vec_still_fails_closed() {
        let h = Harness::new();
        let rendered = h.reject(
            "SI-X09",
            "asan",
            r#"
fn main() -> i64 {
    let mut inner = Vec::new()
    Vec::push(inner, 1)
    let mut vv = Vec::new()
    Vec::push(vv, inner)
    return 0
}
"#,
            OptimizationLevel::None,
        );
        assert!(
            rendered.contains("[E0412]"),
            "nested Vec must fail closed:\n{rendered}"
        );
    }
}

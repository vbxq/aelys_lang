use aelys_air::bir::effects::{EXTERN_DEFAULT, EXTERN_NOGC, effect_summaries_with_imports};
use aelys_air::bir::{
    BirBlock, BirBlockId, BirBody, BirExtern, BirLocalId, BirOperand, BirPlace, BirProgram,
    BirRvalue, BirStmt, BirStmtKind, BirTerminator, Effect, EffectSet,
};
use aelys_driver::{
    LinkRequirement, RuntimeVariant, compile_file_with_llvm_linked,
    compile_file_with_llvm_variant, lower_file_to_air,
};
use aelys_opt::OptimizationLevel;
use aelys_sema::InferType;
use aelys_syntax::{ForeignConv, ForeignDecl, Span};
use std::cell::Cell;
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use tempfile::{TempDir, tempdir};

mod common;
use common::{
    backend_family_code, exe_path_for, exit_code, linker_unavailable, slug, warm_core_archive,
};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const REJECT_LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const ALLOCATORS: &[(&str, Option<&str>)] = &[("immix", None), ("malloc", Some("malloc"))];

struct Outcome {
    exit: i32,
    stdout: String,
    stderr: String,
    stats: Option<(i64, i64)>,
    timed_out: bool,
}

struct Harness {
    dir: TempDir,
    compiled_legs: Cell<usize>,
    linker_skips: Cell<usize>,
}

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
    let (a, m) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, m.trim().parse().ok()?))
}

impl Harness {
    fn new() -> Self {
        warm_core_archive();
        Harness {
            dir: tempdir().expect("tempdir"),
            compiled_legs: Cell::new(0),
            linker_skips: Cell::new(0),
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
            Ok(()) => self.compiled_legs.set(self.compiled_legs.get() + 1),
            Err(err) => {
                if linker_unavailable(&err.to_string()) {
                    common::require_linker_skip(
                        "a skipped value row carries no runtime evidence at all",
                    );
                    self.linker_skips.set(self.linker_skips.get() + 1);
                    return None;
                }
                panic!("{id} at {tag} must compile:\n{src}\nerror: {err}");
            }
        }
        let exe = exe_path_for(&path);
        exe.is_file().then_some(exe)
    }

    fn assert_measured(&self, group: &str) {
        assert!(
            self.compiled_legs.get() > 0,
            "{group}: no executable leg was measured; linker skips={}",
            self.linker_skips.get()
        );
    }

    fn run(&self, exe: &Path, alloc: Option<&str>) -> Outcome {
        self.run_timed(exe, alloc, None)
    }

    fn run_timed(&self, exe: &Path, alloc: Option<&str>, deadline: Option<Duration>) -> Outcome {
        let _pin = common::pin_legs("run_timed", 1);
        let mut cmd = Command::new(exe);
        cmd.env("AELYS_RC_STATS", "1");
        if let Some(a) = alloc {
            cmd.env("AELYS_ALLOC", a);
        }
        let Some(deadline) = deadline else {
            common::note_leg();
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
        common::note_leg();
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
    AllocAgreeOut(i32, &'static str),
    // an exact (allocs, frees) pair beside the value. balanced and namedleak both pass on
    ExitOutStats(i32, &'static str, i64, i64),
    Balanced(i32),
    Leaks(i32, &'static str),
    AllocsAt(&'static str, OptimizationLevel, i32, i64),
}

fn check(h: &Harness, id: &str, src: &str, exit: i32, out: Option<&str>, mem: Memory) {
    let _pin = common::pin_legs(id, LEVELS.len() * ALLOCATORS.len());
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
        Oracle::AllocAgreeOut(e, out) => check_alloc_agree_out(h, id, src, e, out),
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

fn check_alloc_agree_out(h: &Harness, id: &str, src: &str, exit: i32, out: &str) {
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
                "{id} at {name}/{alloc_name}: the answer MUST be {exit}\nsource:{src}\nstdout: {:?}\nstderr:\n{}",
                o.stdout, o.stderr
            );
            assert_eq!(
                o.stdout, out,
                "{id} at {name}/{alloc_name}: stdout MUST be {out:?}\nsource:{src}"
            );
        }
        let (a_name, a) = &legs[0];
        for (b_name, b) in &legs[1..] {
            assert_eq!(
                a.stdout, b.stdout,
                "{id} at {name}: the {a_name} and {b_name} legs MUST agree\nsource:{src}"
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
    twin: Option<(&'static str, i32)>,
}

const GROUP_X: &[XRow] = &[
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
    (
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
        Oracle::AllocAgreeOut(0, "1\n2\n"),
    ),
    (
        // v1: `&v[0]` on a vec compiles and reads right. it never reaches the detach dispatch,
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
    (
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
    (
        "SI-PA34",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33, 44]
    let s = v[0..3]
    println(s[3])
    return 0
}
"#,
        Oracle::ExitOutErr(134, "", "index out of bounds"),
    ),
    (
        "SI-PA35",
        r#"
fn peek(r: &Vec<i64>) -> i64 {
    let s = (*r)[0..3]
    return s[1]
}
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    println(peek(&v))
    return 0
}
"#,
        Oracle::ExitOutStats(0, "22\n", 1, 1),
    ),
    (
        "SI-PA36",
        r#"
fn sum(r: &Vec<i64>) -> i64 {
    let s = (*r)[0..3]
    return s[0] + s[1] + s[2]
}
fn main() -> i64 {
    let v: Vec<i64> = vec[11, 22, 33]
    println(sum(&v))
    return 0
}
"#,
        Oracle::ExitOutStats(0, "66\n", 1, 1),
    ),
    (
        "SI-PA37",
        r#"
struct Cell { x: i64 }
fn read(p: &i64) -> i64 { return *p }
fn main() -> i64 {
    let r = Rc::new(Cell { x: 7 })
    let p = &Rc::get(r).x
    let q = Rc::new(Cell { x: 31 })
    println(read(p) + Rc::get(q).x)
    return 0
}
"#,
        Oracle::ExitOutStats(0, "38\n", 2, 2),
    ),
    (
        "SI-PA38",
        r#"
struct Cell { f: i64 }
fn drain(pr: &mut Cell) -> i64 {
    while (*pr).f > 0 {
        (*pr).f = (*pr).f - 1
    }
    return 0
}
fn main() -> i64 {
    let mut c: Cell = Cell { f: 3 }
    let q = drain(&mut c)
    println(c.f)
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "0\n"),
    ),
    (
        "SI-PA39",
        r#"
struct Ar { a: [i64; 3] }
fn main() -> i64 {
    let mut ar: Ar = Ar { a: [3, 0, 0] }
    let s = ar.a[..]
    while s[0] > 0 {
        s[0] = s[0] - 1
    }
    println(ar.a[0])
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "0\n"),
    ),
    (
        "SI-PA40",
        r#"
struct Cell { f: i64, g: i64 }
fn main() -> i64 {
    let n: Rc<Cell> = Rc::null()
    println(101)
    n.f = 101
    println(7919)
    return 0
}
"#,
        Oracle::ExitOutErr(134, "101\n", "null pointer dereference"),
    ),
    (
        "SI-PA41",
        r#"
struct S { f: i64 }
let mut ga: [i64; 3] = [1901, 2, 3]
let mut gs: S = S { f: 1901 }
fn write_globals() -> i64 {
    ga[0] = 101
    gs.f = 101
    return 0
}
fn main() -> i64 {
    let q = write_globals()
    println(ga[0])
    println(gs.f)
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n101\n"),
    ),
    (
        "SI-PAA01",
        r#"
fn read(s: &[i64]) -> i64 {
    let t = s[0..2]
    return t[1]
}
fn main() -> i64 {
    let a: [i64; 4] = [11, 22, 33, 44]
    return read(a[..])
}
"#,
        Oracle::AllocAgreeOut(22, ""),
    ),
    (
        "SI-PAA02",
        r#"
fn cut(s: &[i64]) -> &[i64] {
    return s[0..2]
}
fn main() -> i64 {
    let a: [i64; 4] = [11, 22, 33, 44]
    let t = cut(a[..])
    return t[1]
}
"#,
        Oracle::AllocAgreeOut(22, ""),
    ),
    (
        "SI-PAA03",
        r#"
fn main() -> i64 {
    let a: [i64; 4] = [11, 22, 33, 44]
    let s = a[..]
    let t = s[0..2]
    println(t[1])
    return 0
}
"#,
        Oracle::AllocAgreeOut(0, "22\n"),
    ),
    (
        "SI-PA46",
        r#"
fn drain(r: &mut i64) -> i64 {
    while *r > 0 {
        (*r)--
    }
    return 0
}
fn main() -> i64 {
    let mut x: i64 = 3
    let q = drain(&mut x)
    println(x)
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "0\n"),
    ),
    (
        "SI-PA47",
        r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [3, 0, 0]
    let r = &mut a
    while (*r)[0] > 0 {
        (*r)[0] = (*r)[0] - 1
    }
    println(a[0])
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "0\n"),
    ),
    (
        "SI-PA48",
        r#"
struct Cell { f: i64 }
fn drain(r: Rc<Cell>) -> i64 {
    while Rc::get(r).f > 0 {
        Rc::get(r).f = Rc::get(r).f - 1
    }
    return 0
}
fn main() -> i64 {
    let r = Rc::new(Cell { f: 3 })
    let q = drain(r)
    return q
}
"#,
        Oracle::Balanced(0),
    ),
    (
        "SI-PA49",
        r#"
fn fill(r: &mut Vec<i64>) -> i64 {
    let mut i = 1
    while i < 4 {
        Vec::push((*r), i)
        i = i + 1
    }
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[0]
    let q = fill(&mut v)
    println(v[3])
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "3\n"),
    ),
    (
        "SI-PA50",
        r#"
fn drain(r: &mut i64) -> i64 {
    if *r > 0 {
        (*r)--
        return drain(r)
    }
    return 0
}
fn main() -> i64 {
    let mut x: i64 = 3
    let q = drain(&mut x)
    println(x)
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "0\n"),
    ),
    (
        "SI-PA51",
        r#"
fn add(r: &mut Vec<i64>) -> i64 {
    Vec::push((*r), 101)
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2]
    let q = add(&mut v)
    println(v[2])
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n"),
    ),
    (
        "SI-PA52",
        r#"
fn bump(s: &mut [i64]) -> i64 {
    s[0]++
    return 0
}
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 2, 3]
    let q = bump(a[..])
    println(a[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "8\n"),
    ),
    (
        "SI-PA53",
        r#"
fn main() -> i64 {
    let v: Vec<i64> = vec[7]
    let f = fn () -> i64 {
        Vec::push(v, 101)
        return v[0]
    }
    let inside = f()
    println(inside)
    println(v[0])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "7\n7\n", 3, 1),
    ),
    (
        "SI-PAA04",
        r#"
fn read(s: &[i64]) -> i64 {
    let t = s[0..3]
    let u = t[0..1]
    return u[0]
}
fn main() -> i64 {
    let a: [i64; 4] = [11, 22, 33, 44]
    return read(a[..])
}
"#,
        Oracle::AllocAgreeOut(11, ""),
    ),
    (
        "SI-PAO02",
        r#"
fn make() -> fn() -> fn() -> i64 {
    let a: i64 = 1
    return fn () -> fn() -> i64 {
        let b: i64 = 2
        return fn () -> i64 { return a + b }
    }
}
fn main() -> i64 {
    let f = make()
    let g = f()
    println(g())
    return 0
}
"#,
        Oracle::ExitOut(0, "3\n"),
    ),
    (
        "SI-PA54",
        r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = b
    return 0
}
"#,
        Oracle::ExitOutStats(0, "", 2, 1),
    ),
    (
        "SI-PA55",
        r#"
fn grow(r: &mut Vec<i64>) -> i64 {
    let q = &mut *r
    Vec::push((*q), 101)
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7]
    let q = grow(&mut v)
    println(v[1])
    return 0
}
"#,
        Oracle::ExitOutStats(0, "101\n", 1, 1),
    ),
    (
        "SI-PA56",
        r#"
fn grow(r: &mut Vec<i64>, i: i64) -> i64 {
    if i < 3 {
        Vec::push((*r), i)
        return grow(r, i + 1)
    }
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[0]
    let q = grow(&mut v, 1)
    println(v[2])
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "2\n"),
    ),
    (
        "SI-PA57",
        r#"
let mut g: i64 = 3
fn drain() -> i64 {
    while g > 0 {
        g--
    }
    return g
}
fn main() -> i64 {
    return drain()
}
"#,
        Oracle::Terminates(20_000, 0, ""),
    ),
    (
        "SI-PA58",
        r#"
struct Cell { a: [i64; 3] }
fn drain(r: &mut Cell) -> i64 {
    while (*r).a[0] > 0 {
        (*r).a[0]--
    }
    return 0
}
fn main() -> i64 {
    let mut c: Cell = Cell { a: [3, 0, 0] }
    let q = drain(&mut c)
    println(c.a[0])
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "0\n"),
    ),
    (
        "SI-PA59",
        r#"
fn main() -> i64 {
    let mut a: [[[i64; 2]; 2]; 2] = [[[1, 2], [3, 4]], [[5, 6], [7, 8]]]
    let r = &mut a
    (*r)[1][1][1] = 101
    return a[1][1][1] - 101
}
"#,
        Oracle::ExitOut(0, ""),
    ),
    (
        "SI-PA60",
        r#"
struct L3 { d: i64 }
struct L2 { c: L3 }
struct L1 { b: L2 }
fn poke(r: &mut L1) -> i64 {
    (*r).b.c.d = 101
    return 0
}
fn main() -> i64 {
    let mut x: L1 = L1 { b: L2 { c: L3 { d: 7 } } }
    let q = poke(&mut x)
    return x.b.c.d - 101
}
"#,
        Oracle::ExitOut(0, ""),
    ),
    (
        "SI-PA61",
        r#"
struct Mid { g: [i64; 1] }
struct Top { f: [Mid; 1] }
fn poke(r: &mut Top) -> i64 {
    (*r).f[0].g[0] = 101
    return 0
}
fn main() -> i64 {
    let mut x: Top = Top { f: [Mid { g: [7] }] }
    let q = poke(&mut x)
    return x.f[0].g[0] - 101
}
"#,
        Oracle::ExitOut(0, ""),
    ),
    (
        "SI-PA63",
        r#"
struct Holder { a: [i64; 3] }
fn down(r: &mut Holder) -> i64 {
    if (*r).a[0] > 0 {
        (*r).a[0] = (*r).a[0] - 1
        let q = down(r)
        return q
    }
    return 0
}
fn main() -> i64 {
    let mut h: Holder = Holder { a: [3, 0, 0] }
    let q = down(&mut h)
    println(h.a[0])
    return q
}
"#,
        Oracle::Terminates(20_000, 0, "0\n"),
    ),
    (
        "SI-PA64",
        r#"
struct Cell { f: i64, g: i64 }
fn down(r: &mut Cell) -> i64 {
    if (*r).f > 0 {
        (*r).f = (*r).f - 1
        let q = down(r)
        return q
    }
    return 0
}
fn main() -> i64 {
    let mut c: Cell = Cell { f: 3, g: 0 }
    let q = down(&mut c)
    println(c.f)
    return q
}
"#,
        Oracle::Terminates(20_000, 0, "0\n"),
    ),
    (
        "SI-PAA05",
        r#"
fn read(s: &[i64]) -> i64 {
    let t = s[0..2]
    return t[0]
}
fn main() -> i64 {
    let a: [i64; 4] = [11, 22, 33, 44]
    return read(a[..])
}
"#,
        Oracle::AllocAgreeOut(11, ""),
    ),
    (
        "SI-PAA06",
        r#"
fn run(s: &mut [i64]) -> i64 {
    let t = s[0..2]
    let mut i = 0
    while i < 1 {
        t[0] = 101
        i = i + 1
    }
    return 0
}
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 0, 0]
    let q = run(a[..])
    println(a[0])
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "101\n"),
    ),
    (
        "SI-PAA07",
        r#"
fn run(s: &mut [i64]) -> i64 {
    let t = s[0..2]
    t[0] = 101
    println(s[0])
    return 0
}
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 0, 0]
    let q = run(a[..])
    println(a[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "101\n101\n"),
    ),
    (
        "SI-PAA08",
        r#"
fn run(s: &mut [i64]) -> i64 {
    let t = s[0..2]
    t[0] = 101
    println(s[0])
    return 0
}
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 0, 0]
    let q = run(a[..])
    println(a[0])
    return q
}
"#,
        Oracle::ExitOut(0, "101\n101\n"),
    ),
];

const GROUP_CA: &[(&str, &str, Oracle)] = &[
    // loop below never advanced and the program hung with no output
    (
        "SI-PACA01",
        r#"
fn main() -> i64 {
    let a: [i64; 2] = [9, 0]
    let mut b: [i64; 2] = [0, 0]
    while b[0] < 8 {
        b[0] = a[0] * 1
    }
    println(b[0])
    return 0
}
"#,
        Oracle::Terminates(20_000, 0, "9\n"),
    ),
    (
        "SI-PACA02",
        r#"
fn main() -> i64 {
    let a: [i64; 2] = [9, 5]
    let mut b: [i64; 2] = [0, 0]
    b[0] = a[0] * 2
    println(b[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA03",
        r#"
fn main() -> i64 {
    let a = vec[9, 5]
    let mut b = vec[0, 0]
    b[0] = a[0] * 2
    println(b[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA04",
        r#"
fn main() -> i64 {
    let mut a: [i64; 2] = [9, 1]
    a[0] = a[1] * 2
    println(a[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "2\n"),
    ),
    // the source subscript was dropped whole, so `side` was never called
    (
        "SI-PACA05",
        r#"
fn side() -> i64 {
    println(77)
    return 0
}
fn main() -> i64 {
    let a: [i64; 2] = [9, 5]
    let mut b: [i64; 2] = [0, 0]
    b[0] = a[side()] * 2
    println(b[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "77\n18\n"),
    ),
    (
        "SI-PACA06",
        r#"
struct P { x: i64 }
fn main() -> i64 {
    let p: P = P { x: 9 }
    let mut q: P = P { x: 0 }
    q.x = p.x * 2
    println(q.x)
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA07",
        r#"
struct Inner { v: i64 }
struct Outer { i: Inner }
fn main() -> i64 {
    let a: Outer = Outer { i: Inner { v: 9 } }
    let mut b: Outer = Outer { i: Inner { v: 0 } }
    b.i.v = a.i.v * 2
    println(b.i.v)
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA08",
        r#"
fn g(v: i64) -> i64 {
    return v
}
fn main() -> i64 {
    let s: [i64; 2] = [9, 5]
    let mut d: [i64; 2] = [0, 0]
    d[0] = g(s[0] * 2)
    println(d[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA09",
        r#"
struct P { d: [i64; 2] }
fn main() -> i64 {
    let p: P = P { d: [9, 5] }
    let mut b: [i64; 2] = [0, 0]
    b[0] = p.d[0] * 2
    println(b[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA10",
        r#"
fn main() -> i64 {
    let a: [i64; 2] = [9, 5]
    let mut b: [i64; 2] = [0, 0]
    let i: i64 = 0
    b[i] = a[i] * 2
    println(b[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA11",
        r#"
fn poke(src: &Vec<i64>, dst: &mut Vec<i64>) -> i64 {
    (*dst)[0] = (*src)[0] * 2
    return 0
}
fn main() -> i64 {
    let s: Vec<i64> = vec[9, 2]
    let mut d: Vec<i64> = vec[0, 0]
    let q = poke(&s, &mut d)
    println(d[0])
    return q
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA12",
        r#"
struct P { x: i64 }
fn main() -> i64 {
    let a: [i64; 2] = [9, 5]
    let mut q: P = P { x: 0 }
    q.x = a[0] * 2
    println(q.x)
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA13",
        r#"
struct P { x: i64 }
fn main() -> i64 {
    let p: P = P { x: 9 }
    let mut b: [i64; 2] = [0, 0]
    b[0] = p.x * 2
    println(b[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA14",
        r#"
fn main() -> i64 {
    let mut b: [i64; 2] = [1, 0]
    b[0] += 3
    println(b[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "4\n"),
    ),
    (
        "SI-PACA15",
        r#"
fn main() -> i64 {
    let mut b: [i64; 2] = [9, 0]
    b[0] = b[0] * 2
    println(b[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA16",
        r#"
fn main() -> i64 {
    let a: [i64; 2] = [9, 5]
    let mut b: [i64; 2] = [0, 0]
    b[0] = 2 * a[0]
    println(b[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA17",
        r#"
struct P { x: i64 }
fn main() -> i64 {
    let mut p: P = P { x: 1 }
    p.x += 3
    println(p.x)
    return 0
}
"#,
        Oracle::ExitOut(0, "4\n"),
    ),
    (
        "SI-PACA18",
        r#"
fn main() -> i64 {
    let a: i64 = 9
    let mut b: i64 = 0
    let pa: &i64 = &a
    let pb: &mut i64 = &mut b
    *pb = *pa * 2
    println(b)
    return 0
}
"#,
        Oracle::ExitOut(0, "18\n"),
    ),
    (
        "SI-PACA19",
        r#"
fn side() -> i64 {
    println(77)
    return 0
}
fn main() -> i64 {
    let mut a: [i64; 2] = [9, 5]
    a[side()] += 3
    println(a[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "77\n12\n"),
    ),
    (
        "SI-PACA20",
        r#"
struct P { d: [i64; 2] }
fn main() -> i64 {
    let mut p: P = P { d: [9, 5] }
    p.d[0] += 3
    println(p.d[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "12\n"),
    ),
    (
        "SI-PACA21",
        r#"
struct Inner { v: i64 }
struct Outer { i: Inner }
fn main() -> i64 {
    let mut b: Outer = Outer { i: Inner { v: 9 } }
    b.i.v += 3
    println(b.i.v)
    return 0
}
"#,
        Oracle::ExitOut(0, "12\n"),
    ),
    (
        "SI-PACA22",
        r#"
fn poke(r: &mut Vec<i64>) -> i64 {
    (*r)[0] += 3
    return 0
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[9, 2]
    let q = poke(&mut v)
    println(v[0])
    return q
}
"#,
        Oracle::ExitOut(0, "12\n"),
    ),
    (
        "SI-PACA23",
        r#"
fn main() -> i64 {
    let mut a: [i64; 2] = [9, 5]
    let i: i64 = 0
    a[i]++
    println(a[0])
    return 0
}
"#,
        Oracle::ExitOut(0, "10\n"),
    ),
    (
        "SI-PACA24",
        r#"
struct P { x: i64 }
fn main() -> i64 {
    let mut p: P = P { x: 9 }
    p.x++
    println(p.x)
    return 0
}
"#,
        Oracle::ExitOut(0, "10\n"),
    ),
];

#[test]
fn group_pa_place_addressing() {
    let h = Harness::new();
    run_rows(&h, GROUP_PA);
    h.assert_measured("group_pa_place_addressing");
}

#[test]
fn group_ca_compound_assignment_is_place_identity() {
    let h = Harness::new();
    run_rows(&h, GROUP_CA);
    h.assert_measured("group_ca_compound_assignment_is_place_identity");
}

// the fail-closed half: one row per code adds, plus the two c52 legs and the origins
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
        // pass as if it borrowed the caller. makes that address real
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
    XRow {
        id: "SI-PAX09",
        code: "E0421",
        rejected: r#"
fn main() -> i64 {
    [337, 0, 0][0] = 101
    return 0
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [337, 0, 0]
    a[0] = 101
    return a[0]
}
"#,
            101,
        )),
    },
    XRow {
        id: "SI-PAX10",
        code: "E0421",
        rejected: r#"
struct Cell { f: i64, g: i64 }
fn main() -> i64 {
    Cell { f: 337, g: 0 }.f = 101
    return 0
}
"#,
        twin: Some((
            r#"
struct Cell { f: i64, g: i64 }
fn main() -> i64 {
    let mut c: Cell = Cell { f: 337, g: 0 }
    c.f = 101
    return c.f
}
"#,
            101,
        )),
    },
    XRow {
        id: "SI-PAX11",
        code: "E0423",
        rejected: r#"
fn main() -> i64 {
    let mut cv: [i64; 3] = [619, 2, 3]
    let f = fn () -> i64 {
        let r = &cv[0]
        cv[0] = 101
        return *r
    }
    return f()
}
"#,
        twin: Some((
            r#"
fn read(p: &i64) -> i64 { return *p }
fn main() -> i64 {
    let x: i64 = 5
    let f = fn (p: &i64) -> i64 { return read(p) }
    return f(&x)
}
"#,
            5,
        )),
    },
    XRow {
        id: "SI-PAX12",
        code: "E0415",
        rejected: r#"
let mut g: [i64; 3] = [1901, 2, 3]
fn main() -> i64 {
    let r = &mut g[0]
    *r = 101
    return g[0]
}
"#,
        twin: Some((
            r#"
let mut g: [i64; 3] = [1901, 2, 3]
fn main() -> i64 {
    g[0] = 101
    return g[0]
}
"#,
            101,
        )),
    },
    XRow {
        id: "SI-PAX13",
        code: "E0422",
        rejected: r#"
fn poke(r: &i64) -> i64 {
    let q = &mut *r
    *q = 101
    return 0
}
fn main() -> i64 {
    let x: i64 = 7
    return poke(&x)
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX14",
        code: "E0422",
        rejected: r#"
fn bump(r: &i64) -> i64 {
    (*r)++
    return 0
}
fn main() -> i64 {
    let x: i64 = 7
    return bump(&x)
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX15",
        code: "E0104",
        rejected: r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 2, 3]
    a[..]++
    return 0
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 2, 3]
    a[0]++
    return a[0]
}
"#,
            8,
        )),
    },
    XRow {
        id: "SI-PAX16",
        code: "E0422",
        rejected: r#"
fn grow(r: &Vec<i64>) -> i64 {
    Vec::push((*r), 101)
    return 0
}
fn main() -> i64 {
    let v: Vec<i64> = vec[7]
    return grow(&v)
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX17",
        code: "E0421",
        rejected: r#"
fn main() -> i64 {
    Vec::push(vec[7], 101)
    return 0
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX18",
        code: "E0421",
        rejected: r#"
struct Cell { f: i64 }
fn main() -> i64 {
    let r = &mut Cell { f: 7 }
    r.f = 101
    return 0
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX20",
        code: "E0419",
        rejected: r#"
enum Vec { Empty, One(i64) }
fn main() -> i64 {
    let x: Vec = Vec::One(7)
    return 0
}
"#,
        twin: Some((
            r#"
enum MyList { Empty, One(i64) }
fn main() -> i64 {
    let x: MyList = MyList::One(7)
    return match x { MyList::One(v) => v, MyList::Empty => 0 }
}
"#,
            7,
        )),
    },
    XRow {
        id: "SI-PAX22",
        code: "E0104",
        rejected: r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 2, 3]
    a[0..2]++
    return 0
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX23",
        code: "E0104",
        rejected: r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 2, 3]
    a[..2]++
    return 0
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX24",
        code: "E0104",
        rejected: r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 2, 3]
    a[0..=2]++
    return 0
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX25",
        code: "E0104",
        rejected: r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 2, 3]
    a[..=2]++
    return 0
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX26",
        code: "E0104",
        rejected: r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 2, 3]
    a[0..0]++
    return 0
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX27",
        code: "E0104",
        rejected: r#"
fn main() -> i64 {
    let mut a: [i64; 3] = [7, 2, 3]
    a[0..1]++
    return 0
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX28",
        code: "E0104",
        rejected: r#"
fn bad(s: &[i64]) -> i64 {
    s[0..2]++
    return 0
}
fn main() -> i64 {
    let a: [i64; 3] = [7, 2, 3]
    return bad(a[..])
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX29",
        code: "E0104",
        rejected: r#"
struct Cell { a: [i64; 3] }
fn main() -> i64 {
    let mut c: Cell = Cell { a: [7, 2, 3] }
    c.a[..]++
    return 0
}
"#,
        twin: None,
    },
    XRow {
        id: "SI-PAX30",
        code: "E0412",
        rejected: r#"
fn main() -> i64 {
    let mut inner = Vec::new()
    Vec::push(inner, 1)
    let mut outer = Vec::new()
    Vec::push(outer, inner)
    return 0
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    Vec::push(v, 2)
    return v[0] + v[1]
}
"#,
            3,
        )),
    },
    XRow {
        id: "SI-PAX31",
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
    let a: [i64; 3] = [1, 2, 3]
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
        id: "SI-PAX32",
        code: "E0417",
        rejected: r#"
fn main() -> i64 {
    let x: i64 = 1
    let a = &mut x
    *a = 2
    return x
}
"#,
        twin: Some((
            r#"
fn main() -> i64 {
    let mut x: i64 = 1
    let a = &mut x
    *a = 2
    return x
}
"#,
            2,
        )),
    },
    XRow {
        id: "SI-PAX33",
        code: "E0418",
        rejected: r#"
fn pick<A, B>(a: A, b: B) -> A { return a }
fn other() -> i64 {
    fn pick<A, B>(a: A, b: B) -> B { return b }
    return pick(1, 2)
}
fn main() -> i64 { return pick(7, 8) }
"#,
        twin: Some((
            r#"
fn pick<A, B>(a: A, b: B) -> A { return a }
fn other() -> i64 {
    fn choose<A, B>(a: A, b: B) -> B { return b }
    return choose(1, 2)
}
fn main() -> i64 { return pick(7, 8) }
"#,
            7,
        )),
    },
    XRow {
        id: "SI-PAX34",
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
        id: "SI-PAX35",
        code: "E0424",
        rejected: r#"
let mut g: i64 = 3
fn drain(r: &mut i64) -> i64 {
    while *r > 0 {
        (*r)--
    }
    return 0
}
fn main() -> i64 {
    let q = drain(&mut g)
    return g
}
"#,
        twin: Some((
            r#"
let mut g: i64 = 3
fn drain() -> i64 {
    while g > 0 {
        g--
    }
    return g
}
fn main() -> i64 { return drain() }
"#,
            0,
        )),
    },
    XRow {
        id: "SI-PAX36",
        code: "E0424",
        rejected: r#"
let mut g: i64 = 3
fn down(r: &mut i64) -> i64 {
    if *r > 0 {
        *r = *r - 1
        let q = down(r)
        return q
    }
    return 0
}
fn main() -> i64 {
    let q = down(&mut g)
    println(g)
    return q
}
"#,
        twin: Some((
            r#"
let mut g: i64 = 3
fn down() -> i64 {
    if g > 0 {
        g = g - 1
        let q = down()
        return q
    }
    return 0
}
fn main() -> i64 {
    return down()
}
"#,
            0,
        )),
    },
    XRow {
        id: "SI-PAO01",
        code: "E0721",
        rejected: r#"
fn bad(x: i64) -> &i64 {
    return &x
}
fn main() -> i64 {
    let r = bad(7)
    return *r
}
"#,
        twin: Some((
            r#"
fn good(x: &i64) -> &i64 {
    return x
}
fn main() -> i64 {
    let x: i64 = 7
    let r = good(&x)
    return *r
}
"#,
            7,
        )),
    },
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PaRetroClass {
    DifferVsAnchor,
    DifferVsPrefix,
    NeitherControl,
}

const PA_RETRO_ANCHOR: &[&str] = &[
    "SI-PA01",
    "SI-PA02",
    "SI-PA03",
    "SI-PA04",
    "SI-PA05",
    "SI-PA07",
    "SI-PA08",
    "SI-PA09",
    "SI-PA09b",
    "SI-PA10",
    "SI-PA11",
    "SI-PA12",
    "SI-PA15",
    "SI-PA17",
    "SI-PA19",
    "SI-PA21",
    "SI-PA22",
    "SI-PA23",
    "SI-PA34",
    "SI-PA35",
    "SI-PA36",
    "SI-PA37",
    "SI-PA38",
    "SI-PA39",
    "SI-PA40",
    "SI-PAA01",
    "SI-PAA02",
    "SI-PAA03",
    "SI-PAA04",
    "SI-PA46",
    "SI-PA47",
    "SI-PA48",
    "SI-PA49",
    "SI-PA50",
    "SI-PA51",
    "SI-PA55",
    "SI-PA56",
    "SI-PA58",
    "SI-PA59",
    "SI-PA60",
    "SI-PA61",
    "SI-PA63",
    "SI-PA64",
    "SI-PAA05",
    "SI-PAA06",
    "SI-PAA07",
    "SI-PAA08",
    "SI-PAX01",
    "SI-PAX02",
    "SI-PAX03",
    "SI-PAX04",
    "SI-PAX05",
    "SI-PAX06",
    "SI-PAX06b",
    "SI-PAX08",
    "SI-PAX09",
    "SI-PAX10",
    "SI-PAX11",
    "SI-PAX13",
    "SI-PAX14",
    "SI-PAX15",
    "SI-PAX16",
    "SI-PAX17",
    "SI-PAX18",
    "SI-PAX22",
    "SI-PAX23",
    "SI-PAX24",
    "SI-PAX25",
    "SI-PAX26",
    "SI-PAX27",
    "SI-PAX28",
    "SI-PAX29",
    "SI-PAX35",
    "SI-PAX36",
    "SI-PAO01",
];

const PA_RETRO_PREFIX: &[&str] = &[
    "SI-PA26", "SI-PA27", "SI-PA28", "SI-PA29", "SI-PA30", "SI-PA31", "SI-PA32", "SI-PA33",
    "SI-PAO02",
];

const PA_RETRO_CONTROLS: &[&str] = &[
    "SI-PA06", "SI-PA13", "SI-PA13b", "SI-PA14", "SI-PA16", "SI-PA18", "SI-PA20", "SI-PA24",
    "SI-PA25", "SI-PA41", "SI-PA52", "SI-PA53", "SI-PA54", "SI-PA57", "SI-PAX07", "SI-PAX12",
    "SI-PAX20", "SI-PAX30", "SI-PAX31", "SI-PAX32", "SI-PAX33", "SI-PAX34",
];

const PA_RETRO_CENSUS: (usize, usize, usize) = (75, 9, 22);

fn pa_retro_class(id: &str) -> Option<PaRetroClass> {
    if PA_RETRO_ANCHOR.contains(&id) {
        Some(PaRetroClass::DifferVsAnchor)
    } else if PA_RETRO_PREFIX.contains(&id) {
        Some(PaRetroClass::DifferVsPrefix)
    } else if PA_RETRO_CONTROLS.contains(&id) {
        Some(PaRetroClass::NeitherControl)
    } else {
        None
    }
}

#[test]
fn group_pa_retro_census_is_tracked() {
    let mut declared = HashSet::new();
    let mut counts = (0, 0, 0);
    for id in PA_RETRO_ANCHOR {
        assert!(declared.insert(*id), "duplicate retro row {id}");
        counts.0 += 1;
        assert_eq!(pa_retro_class(id), Some(PaRetroClass::DifferVsAnchor));
    }
    for id in PA_RETRO_PREFIX {
        assert!(declared.insert(*id), "duplicate retro row {id}");
        counts.1 += 1;
        assert_eq!(pa_retro_class(id), Some(PaRetroClass::DifferVsPrefix));
    }
    for id in PA_RETRO_CONTROLS {
        assert!(declared.insert(*id), "duplicate retro row {id}");
        counts.2 += 1;
        assert_eq!(pa_retro_class(id), Some(PaRetroClass::NeitherControl));
    }
    assert_eq!(counts, PA_RETRO_CENSUS);
    let mut observed = HashSet::new();
    for (id, _, _) in GROUP_PA {
        assert!(observed.insert(*id), "duplicate place row {id}");
        assert!(pa_retro_class(id).is_some(), "unclassified place row {id}");
    }
    for row in GROUP_PA_X {
        assert!(observed.insert(row.id), "duplicate place row {}", row.id);
        assert!(
            pa_retro_class(row.id).is_some(),
            "unclassified place row {}",
            row.id
        );
    }
    assert_eq!(declared, observed);
}

fn rs_sources(dir: &Path, out: &mut Vec<(PathBuf, String)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if path.is_dir() {
            if name == "target" || name.starts_with('.') {
                continue;
            }
            rs_sources(&path, out);
        } else if name.ends_with(".rs") {
            if let Ok(text) = fs::read_to_string(&path) {
                out.push((path, text));
            }
        }
    }
}

fn occurrences(sources: &[&String], needle: &str) -> usize {
    sources
        .iter()
        .map(|text| text.matches(needle).count())
        .sum()
}

#[test]
fn group_pa_choke_set_is_tracked() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("the workspace root is the parent of this crate");

    let mut lower = Vec::new();
    rs_sources(&root.join("air/src/lower"), &mut lower);
    let mut repo = Vec::new();
    rs_sources(root, &mut repo);
    // this file spells out the patterns it counts, so it cannot be inside its own census
    let self_path = root.join(file!());
    repo.retain(|(path, _)| path != &self_path);
    assert_eq!(
        repo.len() + 1,
        {
            let mut everything = Vec::new();
            rs_sources(root, &mut everything);
            everything.len()
        },
        "the self-exclusion matched no file, so the census counts its own patterns"
    );

    assert!(
        lower.len() >= 5 && repo.len() >= 150,
        "the scan fired below magnitude: {} lowering files, {} repo files",
        lower.len(),
        repo.len()
    );
    assert!(
        lower.iter().any(|(path, _)| path.ends_with("place.rs")),
        "place.rs must be in scope for the exclusion to mean anything"
    );

    let all: Vec<&String> = lower.iter().map(|(_, text)| text).collect();
    let outside: Vec<&String> = lower
        .iter()
        .filter(|(path, _)| !path.ends_with("place.rs"))
        .map(|(_, text)| text)
        .collect();
    let repo_all: Vec<&String> = repo.iter().map(|(_, text)| text).collect();

    assert!(
        occurrences(&outside, "Place::") >= 30,
        "the exclusion emptied the scope, only {} `Place::` outside place.rs",
        occurrences(&outside, "Place::")
    );

    assert_eq!(
        (
            occurrences(&outside, "Rvalue::AddressOf("),
            occurrences(&outside, "Place::Field("),
            occurrences(&outside, "Place::Index("),
            occurrences(&all, "Rvalue::Deref("),
            occurrences(&repo_all, "addr_of_own_temp("),
        ),
        (0, 3, 6, 4, 5),
        "the choke set moved; the first three are counted over air/src/lower minus place.rs, \
         `Rvalue::Deref(` over all of air/src/lower, and `addr_of_own_temp(` over the repo minus \
         target as one definition and four callers"
    );
}

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
    h.assert_measured("group_pa_fails_closed");
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
        // the read shape 's interprocedural half must not cost
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
    // the seven rows fenced, now correct programs. the aliased ones carry a divergence as
    (
        "SI-X01",
        r#"
fn main() -> i64 {
    let mut v = vec[1, 2, 3]
    let s = v[0..2]
    s[0] = 99
    return v[0]
}
"#,
        Oracle::ExitOutStats(99, "", 1, 1),
    ),
    (
        "S2-M01",
        r#"
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
        Oracle::ExitOutStats(0, "99\n1\n", 2, 2),
    ),
    (
        "S2-M02",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let s = v[0..2]
    let t = s
    t[0] = 99
    return v[0]
}
"#,
        Oracle::ExitOutStats(99, "", 1, 1),
    ),
    (
        "S2-M03",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let s = v[0..3]
    let u = s[0..2]
    u[0] = 99
    return v[0]
}
"#,
        Oracle::ExitOutStats(99, "", 1, 1),
    ),
    (
        "S2-M11",
        r#"
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
        Oracle::ExitOutStats(0, "11\n99\n", 2, 2),
    ),
    (
        "S2-M12",
        r#"
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
        Oracle::ExitOutStats(0, "11\n", 2, 2),
    ),
    (
        // a re-slice inside the callee, so the write's dest is two borrows from the referent
        "S2-M14",
        r#"
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
        Oracle::ExitOutStats(0, "11\n", 2, 2),
    ),
    (
        "S2-M16",
        r#"
struct Ar { a: [i64; 3] }
fn poke(r: &mut Ar) -> i64 {
    let s = (*r).a[0..3]
    s[0] = 99
    return 0
}
fn main() -> i64 {
    let mut b = Ar { a: [11, 22, 33] }
    let z = poke(&mut b)
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
        id: "S2-M04",
        code: "E0422",
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
        code: "E0422",
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
fn d(s: &mut [i64]) -> i64 {
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
        code: "E0422",
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
        id: "S2-M13",
        code: "E0422",
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
        code: "E0902",
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
    let info =
        registry::lookup("E0425").unwrap_or_else(|| panic!("E0425 must have an --explain entry"));
    assert!(
        !info.explanation.trim().is_empty(),
        "E0425's --explain entry must not be empty"
    );
    assert!(
        registry::lookup("E0426").is_none(),
        "E0426 was E0413's other heir and was discharged at the formation of the view; if it is \
         back, the succession has three live members and this record is wrong"
    );
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

// substitution so a hand-copied twin cannot drift away from the program it twins
const DECL_MARKER: &str = "@DECL@";
const NOGC_VIOLATING_DECL: &str = "nogc fn bad() -> Vec<i64> { return Vec::new() }";
const NOGC_CLEAN_DECL: &str = "nogc fn bad() -> i64 { return 42 }";

struct NRow {
    id: &'static str,
    template: &'static str,
}

impl NRow {
    fn rejected(&self) -> String {
        self.template.replace(DECL_MARKER, NOGC_VIOLATING_DECL)
    }

    fn twin(&self) -> String {
        self.template.replace(DECL_MARKER, NOGC_CLEAN_DECL)
    }
}

const GROUP_N_X: &[NRow] = &[
    NRow {
        id: "SI-N00",
        template: r#"@DECL@
fn main() -> i64 {
    return 0
}
"#,
    },
    NRow {
        id: "SI-N01",
        template: r#"fn host() -> i64 {
    @DECL@
    return 0
}
fn main() -> i64 {
    return host()
}
"#,
    },
    NRow {
        id: "SI-N02",
        template: r#"fn host() -> i64 {
    {
        @DECL@
    }
    return 0
}
fn main() -> i64 { return host() }
"#,
    },
    NRow {
        id: "SI-N03",
        template: r#"fn host(c: bool) -> i64 {
    if c {
        @DECL@
    }
    return 0
}
fn main() -> i64 { return host(true) }
"#,
    },
    NRow {
        id: "SI-N04",
        template: r#"fn host(c: bool) -> i64 {
    if c {
        return 0
    } else {
        @DECL@
    }
    return 0
}
fn main() -> i64 { return host(true) }
"#,
    },
    NRow {
        id: "SI-N05",
        template: r#"fn host(n: i64) -> i64 {
    let mut i = 0
    while i < n {
        @DECL@
        i = i + 1
    }
    return 0
}
fn main() -> i64 { return host(1) }
"#,
    },
    NRow {
        id: "SI-N06",
        template: r#"fn host(n: i64) -> i64 {
    for i in 0..n {
        @DECL@
    }
    return 0
}
fn main() -> i64 { return host(1) }
"#,
    },
    NRow {
        id: "SI-N07",
        template: r#"fn host(a: [i64; 2]) -> i64 {
    for x in a {
        @DECL@
    }
    return 0
}
fn main() -> i64 {
    let a: [i64; 2] = [1, 2]
    return host(a)
}
"#,
    },
    NRow {
        id: "SI-N08",
        template: r#"fn host(c: bool, d: bool) -> i64 {
    if c {
        if d {
            @DECL@
        }
    }
    return 0
}
fn main() -> i64 { return host(true, true) }
"#,
    },
    NRow {
        id: "SI-N09",
        template: r#"fn host(c: bool, n: i64) -> i64 {
    if c {
        let mut i = 0
        while i < n {
            @DECL@
            i = i + 1
        }
    }
    return 0
}
fn main() -> i64 { return host(true, 1) }
"#,
    },
    NRow {
        id: "SI-N10",
        template: r#"fn host() -> i64 {
    fn mid() -> i64 {
        {
            @DECL@
        }
        return 0
    }
    return mid()
}
fn main() -> i64 { return host() }
"#,
    },
    NRow {
        id: "SI-N11",
        template: r#"{
    @DECL@
}
fn main() -> i64 { return 0 }
"#,
    },
    NRow {
        id: "SI-N12",
        template: r#"fn mk(x: i64) -> bool {
    if x > 0 {
        return true
    }
    return false
}
if mk(1) {
    @DECL@
}
fn main() -> i64 { return 0 }
"#,
    },
    NRow {
        id: "SI-N13",
        template: r#"fn host() -> i64 {
    let f = fn (x: i64) -> i64 {
        @DECL@
        return x
    }
    return f(0)
}
fn main() -> i64 { return host() }
"#,
    },
    NRow {
        id: "SI-N14",
        template: r#"fn host(n: i64) -> i64 {
    let mut i = 0
    while i < { @DECL@  n } {
        i = i + 1
    }
    return 0
}
fn main() -> i64 { return host(1) }
"#,
    },
    NRow {
        id: "SI-N15",
        template: r#"fn host(n: i64) -> i64 {
    for i in 0..{ @DECL@  n } {
        let q = i
    }
    return 0
}
fn main() -> i64 { return host(1) }
"#,
    },
    NRow {
        id: "SI-N16",
        template: r#"fn host(n: i64) -> i64 {
    for i in { @DECL@  0 }..n {
        let q = i
    }
    return 0
}
fn main() -> i64 { return host(1) }
"#,
    },
    NRow {
        id: "SI-N17",
        template: r#"fn host(a: [i64; 2]) -> i64 {
    for x in { @DECL@  a } {
        let q = x
    }
    return 0
}
fn main() -> i64 {
    let a: [i64; 2] = [1, 2]
    return host(a)
}
"#,
    },
    NRow {
        id: "SI-N18",
        template: r#"fn take(x: i64) -> i64 { return x }
fn host(n: i64) -> i64 {
    return take({ @DECL@  n })
}
fn main() -> i64 { return host(0) }
"#,
    },
    NRow {
        id: "SI-N19",
        template: r#"fn host(n: i64) -> i64 {
    return { @DECL@  n }
}
fn main() -> i64 { return host(0) }
"#,
    },
    NRow {
        id: "SI-N20",
        template: r#"fn host(n: i64) -> i64 {
    let v = { @DECL@  n }
    return v
}
fn main() -> i64 { return host(0) }
"#,
    },
    NRow {
        id: "SI-N21",
        template: r#"enum Pick { One, Two }
fn host(p: Pick, n: i64) -> i64 {
    let v = match p {
        Pick::One => { @DECL@  n },
        Pick::Two => n,
    }
    return v
}
fn main() -> i64 { return host(Pick::One, 0) }
"#,
    },
    NRow {
        id: "SI-N22",
        template: r#"enum Pick { One, Two }
fn host(p: Pick, n: i64) -> i64 {
    let v = match { @DECL@  p } {
        Pick::One => n,
        Pick::Two => n,
    }
    return v
}
fn main() -> i64 { return host(Pick::One, 0) }
"#,
    },
    NRow {
        id: "SI-N23",
        template: r#"fn host(c: bool, n: i64) -> i64 {
    let v = if c { @DECL@  n } else { n }
    return v
}
fn main() -> i64 { return host(true, 0) }
"#,
    },
    NRow {
        id: "SI-N24",
        template: r#"fn host(n: i64) -> i64 {
    let v = unsafe { @DECL@  n }
    return v
}
fn main() -> i64 { return host(0) }
"#,
    },
    NRow {
        id: "SI-N25",
        template: r#"fn host(n: i64) -> i64 {
    let v = n + { @DECL@  0 }
    return v
}
fn main() -> i64 { return host(0) }
"#,
    },
    NRow {
        id: "SI-N26",
        template: r#"fn host(n: i64) -> i64 {
    let a: [i64; 2] = [{ @DECL@  n }, n]
    return a[0]
}
fn main() -> i64 { return host(0) }
"#,
    },
    NRow {
        id: "SI-N27",
        template: r#"fn host(n: i64) -> i64 {
    let a: [i64; 2] = [n, n]
    return a[{ @DECL@  0 }]
}
fn main() -> i64 { return host(0) }
"#,
    },
    NRow {
        id: "SI-N28",
        template: r#"fn host(n: i64) -> i64 {
    println("{ 0 + { @DECL@  n } }")
    return 0
}
fn main() -> i64 { return host(0) }
"#,
    },
    NRow {
        id: "SI-N29",
        template: r#"fn host(n: i64) -> i64 {
    let f = fn (x: i64) -> i64 { return x + { @DECL@  0 } }
    return f(n)
}
fn main() -> i64 { return host(0) }
"#,
    },
];

#[test]
fn group_n_block_nested_fails_closed() {
    let h = Harness::new();
    let mut seen = HashSet::new();
    for row in GROUP_N_X {
        assert!(seen.insert(row.id), "duplicate block-form row {}", row.id);
        assert!(
            row.template.contains(DECL_MARKER),
            "{}: the template has no declaration site, so this row proves nothing",
            row.id
        );
        let rejected = row.rejected();
        let twin = row.twin();
        assert_ne!(
            rejected, twin,
            "{}: the twin must differ from the program it twins",
            row.id
        );
        for (name, opt) in REJECT_LEVELS {
            let rendered = h.reject(row.id, name, &rejected, *opt);
            assert!(
                rendered.contains("[E0727]"),
                "{} at {name} MUST be rejected with E0727, got:\n{rendered}",
                row.id
            );
        }
        let twin_id = format!("{}-twin", row.id);
        for (name, opt) in REJECT_LEVELS {
            h.accepts(&twin_id, name, &twin, *opt);
        }
    }
    assert_eq!(seen.len(), 30, "the block-form enumeration lost a row");
}

fn marker_lines(rendered: &str, marker: char) -> usize {
    rendered
        .lines()
        .filter_map(|line| line.splitn(2, '|').nth(1))
        .filter(|tail| tail.trim_start().starts_with(marker))
        .count()
}

#[test]
fn n30_named_hop_chain_replaces_the_indirect_call() {
    let h = Harness::new();
    let src = r#"
fn host() -> i64 {
    {
        fn allocs() -> Vec<i64> {
            return Vec::new()
        }
        nogc fn caller() -> i64 {
            let v = allocs()
            return 0
        }
    }
    return 0
}
fn main() -> i64 { return host() }
"#;
    for (name, opt) in REJECT_LEVELS {
        let rendered = h.reject("SI-N30", name, src, *opt);
        assert!(
            rendered.contains("[E0727]"),
            "SI-N30 at {name} MUST be rejected with E0727, got:\n{rendered}"
        );
        assert!(
            rendered.contains("caller -> allocs -> Vec::new"),
            "SI-N30 at {name} MUST name every hop, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("<indirect call>"),
            "SI-N30 at {name} MUST NOT fall back to an indirect call, got:\n{rendered}"
        );
    }
}

#[test]
fn n40_format_string_interpolation_is_named_not_caret_accurate() {
    let h = Harness::new();
    let src = r#"
fn host(n: i64) -> i64 {
    println("{ 0 + { nogc fn bad() -> Vec<i64> { return Vec::new() }  n } }")
    return 0
}
fn main() -> i64 { return host(0) }
"#;
    for (name, opt) in REJECT_LEVELS {
        let rendered = h.reject("SI-N40", name, src, *opt);
        assert!(
            rendered.contains("[E0727]")
                && rendered.contains("`bad` is declared nogc")
                && rendered.contains("bad -> Vec::new"),
            "SI-N40 at {name} MUST name the interpolated declaration, got:\n{rendered}"
        );
    }
}

const N50_SRC: &str = r#"
fn f() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    return 1
}
fn g() -> i64 {
    {
        nogc fn f() -> i64 { return 99 }
    }
    return 0
}
fn main() -> i64 {
    println(f())
    println(g())
    return 0
}
"#;

#[test]
fn n50_declared_over_rejection_is_e0727() {
    let h = Harness::new();
    for (name, opt) in REJECT_LEVELS {
        let rendered = h.reject("SI-N50", name, N50_SRC, *opt);
        assert!(
            rendered.contains("[E0727]"),
            "SI-N50 at {name} MUST be rejected with E0727, got:\n{rendered}"
        );
    }
}

#[test]
fn n60_e0418_renders_exactly_one_label() {
    let h = Harness::new();
    let src = r#"
fn dup() -> i64 { return 1 }
fn outer() -> i64 {
    fn dup() -> i64 {
        let a = 1
        let b = 2
        let c = 3
        return a + b + c
    }
    return dup()
}
fn main() -> i64 {
    println(outer())
    return 0
}
"#;
    for (name, opt) in REJECT_LEVELS {
        let rendered = h.reject("SI-N60", name, src, *opt);
        assert!(
            rendered.contains("[E0418]"),
            "SI-N60 at {name} MUST be rejected with E0418, got:\n{rendered}"
        );
        assert_eq!(
            marker_lines(&rendered, '^'),
            1,
            "SI-N60 at {name} MUST draw exactly one label, got:\n{rendered}"
        );
    }
}

const GROUP_N: &[(&str, &str, Oracle)] = &[
    (
        "SI-N70",
        r#"
fn host(n: i64) -> i64 {
    let mut i = 0
    let mut r = 0
    while i < n {
        nogc fn clean() -> i64 { return 42 }
        r = clean()
        i = i + 1
    }
    return r
}
fn main() -> i64 {
    println(host(1))
    return 0
}
"#,
        Oracle::ExitOut(0, "42\n"),
    ),
    (
        "SI-N71",
        r#"
fn host(n: i64) -> i64 {
    let mut i = 0
    let mut r = 0
    while i < n {
        fn grow() -> i64 {
            let mut v = Vec::new()
            Vec::push(v, 1)
            return v[0]
        }
        r = grow()
        i = i + 1
    }
    return r
}
fn main() -> i64 {
    println(host(1))
    return 0
}
"#,
        Oracle::ExitOutStats(0, "1\n", 1, 1),
    ),
    (
        "SI-N72",
        r#"
fn host() -> i64 {
    {
        fn make() -> Vec<i64> {
            let mut v = Vec::new()
            Vec::push(v, 1)
            return v
        }
        let w = make()
        println(w[0])
    }
    return 0
}
fn main() -> i64 { return host() }
"#,
        Oracle::ExitOutStats(0, "1\n", 1, 1),
    ),
];

#[test]
fn group_n_block_nested_runs() {
    let h = Harness::new();
    run_rows(&h, GROUP_N);
    h.assert_measured("group_n_block_nested_runs");
}

struct SymRow {
    id: &'static str,
    code: &'static str,
    src: &'static str,
    // the fence replaces a backend failure, so no member of the backend family may remain
    no_backend_error: bool,
}

// base behaviour is recorded per row because none of it survives the fence: these programs are
const GROUP_N_SYM: &[SymRow] = &[
    SymRow {
        // base prints 1 then 1: the second body is never emitted
        id: "SI-S01",
        code: "E0427",
        src: r#"
fn p() -> i64 {
    fn a() -> i64 { return 1 }
    return a()
}
fn q() -> i64 {
    fn a() -> i64 { return 2 }
    return a()
}
fn main() -> i64 {
    println(p())
    println(q())
    return 0
}
"#,
        no_backend_error: false,
    },
    SymRow {
        id: "SI-S02",
        code: "E0427",
        src: r#"
fn dup() -> i64 { return 0 }
fn outer() -> i64 {
    {
        fn dup() -> i64 { return 5 }
        return dup()
    }
}
fn main() -> i64 {
    println(outer())
    return 0
}
"#,
        no_backend_error: false,
    },
    SymRow {
        id: "SI-S03",
        code: "E0427",
        src: r#"
fn dup(x: i64) -> i64 { return x }
fn outer() -> i64 {
    {
        fn dup() -> i64 { return 5 }
        return dup()
    }
}
fn main() -> i64 {
    println(outer())
    return 0
}
"#,
        no_backend_error: true,
    },
    SymRow {
        id: "SI-S04",
        code: "E0427",
        src: r#"
fn f() -> i64 {
    let mut v = Vec::new()
    Vec::push(v, 1)
    return 1
}
fn g() -> i64 {
    {
        fn f() -> i64 { return 99 }
    }
    return 0
}
fn main() -> i64 {
    println(f())
    println(g())
    return 0
}
"#,
        no_backend_error: false,
    },
    SymRow {
        id: "SI-S10",
        code: "E0428",
        src: r#"
fn __aelys_main() -> i64 {
    println(111)
    return 0
}
fn main() -> i64 {
    println(222)
    return __aelys_main()
}
"#,
        no_backend_error: false,
    },
    SymRow {
        id: "SI-S11",
        code: "E0428",
        src: r#"
fn main() -> i64 {
    println(222)
    return __aelys_main()
}
fn __aelys_main() -> i64 {
    println(111)
    return 0
}
"#,
        no_backend_error: false,
    },
    SymRow {
        id: "SI-S12",
        code: "E0428",
        src: r#"
fn __aelys_user_main() -> i64 {
    println(111)
    return 0
}
fn main() -> i64 {
    println(222)
    return __aelys_user_main()
}
"#,
        no_backend_error: true,
    },
    SymRow {
        // base prints 2 then 2. this row fails if the pre-monomorphization placement is dropped:
        id: "SI-S20",
        code: "E0427",
        src: r#"
fn o1() -> i64 {
    {
        fn g<T>(x: T) -> i64 { return 1 }
        return g(1)
    }
}
fn o2() -> i64 {
    {
        fn g<T>(x: T) -> i64 { return 2 }
        return g(1)
    }
}
fn main() -> i64 {
    println(o1())
    println(o2())
    return 0
}
"#,
        no_backend_error: false,
    },
    SymRow {
        id: "SI-S21",
        code: "E0427",
        src: r#"
fn o1() -> i64 {
    {
        fn g(x: i64) -> i64 { return 1 }
        return g(1)
    }
}
fn o2() -> i64 {
    {
        fn g(x: i64) -> i64 { return 2 }
        return g(1)
    }
}
fn main() -> i64 {
    println(o1())
    println(o2())
    return 0
}
"#,
        no_backend_error: false,
    },
    SymRow {
        id: "SI-S22",
        code: "E0428",
        src: r#"
fn __mono_ident_i64(x: i64) -> i64 { return 105 }
fn ident<T>(x: T) -> T { return x }
fn main() -> i64 {
    println(__mono_ident_i64(1))
    println(ident(105))
    return 0
}
"#,
        no_backend_error: false,
    },
    SymRow {
        id: "SI-S23",
        code: "E0428",
        src: r#"
fn first() -> i64 {
    let f = fn (x: i64) -> i64 { return x }
    return f(5)
}
fn __lambda_1(x: i64) -> i64 { return 77 }
fn main() -> i64 {
    println(first())
    println(__lambda_1(77))
    return 0
}
"#,
        no_backend_error: true,
    },
    SymRow {
        // this is the one collision shape that reaches the fence without pre-empting it
        id: "SI-S25",
        code: "E0427",
        src: r#"
fn dup() -> i64 { return 0 }
fn holder() -> i64 {
    let g = fn() -> i64 {
        fn dup() -> i64 { return 5 }
        return 0
    }
    return g()
}
fn main() -> i64 {
    println(dup())
    println(holder())
    return 0
}
"#,
        no_backend_error: false,
    },
    SymRow {
        // mono joins the name and its type arguments with `_`, so `f<a_b>` and `f_a<b>` mangle
        id: "SI-S24",
        code: "E0427",
        src: r#"
struct B { v: i64 }
struct A_B { v: i64 }
fn f<T>(x: T) -> i64 { return 1 }
fn f_A<T>(x: T) -> i64 { return 2 }
fn main() -> i64 {
    let b = B { v: 0 }
    let ab = A_B { v: 0 }
    println(f(ab))
    println(f_A(b))
    return 0
}
"#,
        no_backend_error: true,
    },
    SymRow {
        id: "L1",
        code: "E0428",
        src: r#"
fn __helper(x: i64) -> i64 { return x * 2 }
fn main() -> i64 {
    println(__helper(21))
    return 0
}
"#,
        no_backend_error: false,
    },
];

const GROUP_N_SYM_CONTROLS: &[(&str, &str, Oracle)] = &[
    (
        "SI-S00",
        r#"
fn outer() -> i64 {
    fn a() -> i64 { return 1 }
    fn b() -> i64 { return 2 }
    println(a())
    println(b())
    return 0
}
fn main() -> i64 { return outer() }
"#,
        Oracle::ExitOut(0, "1\n2\n"),
    ),
    (
        "SI-S11ctl",
        r#"
fn main() -> i64 {
    println(222)
    return helper()
}
fn helper() -> i64 {
    println(111)
    return 0
}
"#,
        Oracle::Terminates(5000, 0, "222\n111\n"),
    ),
    (
        "SI-S20ctl",
        r#"
fn o1() -> i64 {
    {
        fn ga<T>(x: T) -> i64 { return 1 }
        return ga(1)
    }
}
fn o2() -> i64 {
    {
        fn gb<T>(x: T) -> i64 { return 2 }
        return gb(1)
    }
}
fn main() -> i64 {
    println(o1())
    println(o2())
    return 0
}
"#,
        Oracle::ExitOut(0, "1\n2\n"),
    ),
    (
        "SI-S23ctl",
        r#"
fn first() -> i64 {
    let f = fn (x: i64) -> i64 { return x }
    return f(5)
}
fn helper(x: i64) -> i64 { return 77 }
fn main() -> i64 {
    println(first())
    println(helper(77))
    return 0
}
"#,
        Oracle::ExitOut(0, "5\n77\n"),
    ),
    (
        "L1ctl",
        r#"
fn helper(x: i64) -> i64 { return x * 2 }
fn main() -> i64 {
    println(helper(21))
    return 0
}
"#,
        Oracle::ExitOut(0, "42\n"),
    ),
];

#[test]
fn group_n_symbol_identity_fails_closed() {
    let h = Harness::new();
    let mut seen = HashSet::new();
    for row in GROUP_N_SYM {
        assert!(seen.insert(row.id), "duplicate symbol row {}", row.id);
        for (name, opt) in REJECT_LEVELS {
            let rendered = h.reject(row.id, name, row.src, *opt);
            assert!(
                rendered.contains(&format!("[{}]", row.code)),
                "{} at {name} MUST be rejected with {}, got:\n{rendered}",
                row.id,
                row.code
            );
            if row.no_backend_error {
                let leaked = backend_family_code(&rendered);
                assert!(
                    leaked.is_none(),
                    "{} at {name} MUST no longer reach the backend family, got {leaked:?} in:\n{rendered}",
                    row.id
                );
            }
        }
    }
    run_rows(&h, GROUP_N_SYM_CONTROLS);
    h.assert_measured("group_n_symbol_identity_fails_closed");
}

#[test]
fn s05_e0427_renders_one_label_per_definition() {
    let h = Harness::new();
    let src = GROUP_N_SYM
        .iter()
        .find(|row| row.id == "SI-S20")
        .expect("SI-S20 must exist")
        .src;
    for (name, opt) in REJECT_LEVELS {
        let rendered = h.reject("SI-S05", name, src, *opt);
        assert!(
            rendered.contains("[E0427]"),
            "SI-S05 at {name} MUST be rejected with E0427, got:\n{rendered}"
        );
        assert_eq!(
            (marker_lines(&rendered, '^'), marker_lines(&rendered, '-')),
            (1, 1),
            "SI-S05 at {name} MUST draw exactly one label per definition, got:\n{rendered}"
        );
        assert!(
            rendered.contains("inside `o1`") && rendered.contains("inside `o2`"),
            "SI-S05 at {name} MUST name both parents, got:\n{rendered}"
        );
    }
}

#[test]
fn s07_e0427_sees_through_a_lambda_body_and_names_the_parent() {
    let h = Harness::new();
    let src = GROUP_N_SYM
        .iter()
        .find(|row| row.id == "SI-S25")
        .expect("SI-S25 must exist")
        .src;
    for (name, opt) in REJECT_LEVELS {
        let rendered = h.reject("SI-S07", name, src, *opt);
        assert!(
            rendered.contains("same symbol `dup`"),
            "SI-S07 at {name} MUST name the colliding symbol, got:\n{rendered}"
        );
        assert_eq!(
            (marker_lines(&rendered, '^'), marker_lines(&rendered, '-')),
            (1, 1),
            "SI-S07 at {name} MUST draw one label per definition, got:\n{rendered}"
        );
        assert!(
            rendered.contains("defined here, inside `holder`"),
            "SI-S07 at {name} MUST resolve the parent through the lambda body, got:\n{rendered}"
        );
    }
}

#[test]
fn s06_e0427_names_the_mangled_symbol_and_both_air_spans() {
    let h = Harness::new();
    let src = GROUP_N_SYM
        .iter()
        .find(|row| row.id == "SI-S24")
        .expect("SI-S24 must exist")
        .src;
    for (name, opt) in REJECT_LEVELS {
        let rendered = h.reject("SI-S06", name, src, *opt);
        assert!(
            rendered.contains("same symbol `__mono_f_A_B`"),
            "SI-S06 at {name} MUST name the mangled symbol, got:\n{rendered}"
        );
        assert_eq!(
            (marker_lines(&rendered, '^'), marker_lines(&rendered, '-')),
            (1, 1),
            "SI-S06 at {name} MUST draw one label per definition, got:\n{rendered}"
        );
        assert!(
            !rendered.contains("inside `"),
            "SI-S06 at {name} has no typed declaration for the symbol, so no parent can be \
             named, got:\n{rendered}"
        );
        assert!(
            rendered.contains("type arguments joined by"),
            "SI-S06 at {name} MUST explain the mangling, not nesting, got:\n{rendered}"
        );
    }
}

#[test]
fn s2_run3_codes_are_registered() {
    use aelys_common::diagnostic::registry;
    for code in ["E0427", "E0428"] {
        let info =
            registry::lookup(code).unwrap_or_else(|| panic!("{code} must have an --explain entry"));
        assert!(
            !info.explanation.trim().is_empty(),
            "{code}'s --explain entry must not be empty"
        );
    }
}

const O_LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

#[derive(Clone, PartialEq, Eq, Debug)]
enum Verdict {
    Accepted,
    Rejected(String),
}

// an uncoded rejection is its own bucket, so a renderer change cannot make two different
fn first_code(rendered: &str) -> String {
    let bytes = rendered.as_bytes();
    for i in 0..bytes.len().saturating_sub(6) {
        if bytes[i] == b'['
            && bytes[i + 1] == b'E'
            && bytes[i + 2..i + 6].iter().all(u8::is_ascii_digit)
            && bytes[i + 6] == b']'
        {
            return rendered[i + 1..i + 6].to_string();
        }
    }
    "UNCODED".to_string()
}

fn verdict_at(h: &Harness, id: &str, tag: &str, src: &str, opt: OptimizationLevel) -> Verdict {
    let path = h.write(id, tag, src);
    match lower_file_to_air(&path, opt) {
        Ok(_) => Verdict::Accepted,
        Err(rendered) => Verdict::Rejected(first_code(&rendered)),
    }
}

fn verdicts_agree(verdicts: &[Verdict]) -> bool {
    verdicts.windows(2).all(|pair| pair[0] == pair[1])
}

const GROUP_N_DIV: &[(&str, &str)] = &[
    (
        "SI-D01",
        r#"
fn main() -> i64 {
    let c = 1 > 0
    if c {
        nogc fn bad() -> Vec<i64> { return Vec::new() }
    }
    return 0
}
"#,
    ),
    (
        "SI-D02",
        r#"
fn main() -> i64 {
    let c = 1 > 2
    if c {
        let q = 1
    } else {
        nogc fn bad() -> Vec<i64> { return Vec::new() }
    }
    return 0
}
"#,
    ),
    (
        "SI-D03",
        r#"
fn main() -> i64 {
    if true {
        nogc fn bad() -> Vec<i64> { return Vec::new() }
    }
    return 0
}
"#,
    ),
    (
        "SI-D04",
        r#"
fn host() -> i64 {
    let c = 2 > 1
    if c {
        nogc fn bad() -> Vec<i64> { return Vec::new() }
    }
    return 0
}
fn main() -> i64 { return host() }
"#,
    ),
    (
        "SI-D05",
        r#"
fn main() -> i64 {
    let c = 1 > 0
    if c {
        if c {
            nogc fn bad() -> Vec<i64> { return Vec::new() }
        }
    }
    return 0
}
"#,
    ),
];

#[test]
fn group_n_o_invariance() {
    let unequal = [
        Verdict::Accepted,
        Verdict::Accepted,
        Verdict::Rejected("E0727".to_string()),
        Verdict::Rejected("E0727".to_string()),
    ];
    assert!(
        !verdicts_agree(&unequal),
        "the comparator must detect a divergent 4-tuple"
    );
    let unequal_codes = [
        Verdict::Rejected("E0901".to_string()),
        Verdict::Rejected("E0428".to_string()),
    ];
    assert!(
        !verdicts_agree(&unequal_codes),
        "the comparator must detect two different refusals"
    );
    assert!(verdicts_agree(&[Verdict::Accepted, Verdict::Accepted]));

    let h = Harness::new();
    let mut programs: Vec<(String, String)> = Vec::new();
    for row in GROUP_N_X {
        programs.push((row.id.to_string(), row.rejected()));
        programs.push((format!("{}-twin", row.id), row.twin()));
    }
    for row in GROUP_N_SYM {
        programs.push((row.id.to_string(), row.src.to_string()));
    }
    for (id, src, _) in GROUP_N_SYM_CONTROLS {
        programs.push((id.to_string(), src.to_string()));
    }
    for (id, src) in GROUP_N_DIV {
        programs.push((id.to_string(), src.to_string()));
    }
    assert!(
        programs.len() >= 80,
        "the invariance sweep swept only {} programs",
        programs.len()
    );

    let mut divergent = Vec::new();
    for (id, src) in &programs {
        let verdicts: Vec<Verdict> = O_LEVELS
            .iter()
            .map(|(tag, opt)| verdict_at(&h, id, tag, src, *opt))
            .collect();
        if !verdicts_agree(&verdicts) {
            divergent.push(format!("{id}: {verdicts:?}"));
        }
    }
    assert!(
        divergent.is_empty(),
        "every verdict MUST be the same at every optimization level; divergent:\n{}",
        divergent.join("\n")
    );
}

const STAGE3_UNIQUE_MUT_SLICE: &str = r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    let mut s: &mut [i64] = Vec::try_as_unique_mut_slice(v)
    s[0] = 101
    println(v[0])
    return 0
}
"#;

const STAGE3_SHARED_MUT_SLICE: &str = r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    let alias = v
    let mut s: &mut [i64] = Vec::try_as_unique_mut_slice(v)
    s[0] = 101
    println(alias[0])
    return 0
}
"#;

const STAGE3_NOGC_MUT_SLICE: &str = r#"
nogc fn view(r: &mut Vec<i64>) -> &mut [i64] {
    return Vec::try_as_unique_mut_slice(*r)
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    let mut s: &mut [i64] = view(&mut v)
    s[0] = 101
    println(v[0])
    return 0
}
"#;

#[test]
fn stage3_unique_mut_slice_writes_only_a_unique_buffer() {
    let h = Harness::new();
    run_row(
        &h,
        "S3-UMS-unique",
        STAGE3_UNIQUE_MUT_SLICE,
        Oracle::ExitOutStats(0, "101\n", 1, 1),
    );
    h.assert_measured("S3-UMS-unique");
}

#[test]
fn stage3_unique_mut_slice_shared_buffer_is_an_empty_view() {
    let h = Harness::new();
    run_row(
        &h,
        "S3-UMS-shared",
        STAGE3_SHARED_MUT_SLICE,
        Oracle::ExitOut(134, ""),
    );
    h.assert_measured("S3-UMS-shared");
}

#[test]
fn stage3_unique_mut_slice_is_effect_free_inside_nogc() {
    let h = Harness::new();
    run_row(
        &h,
        "S3-UMS-nogc",
        STAGE3_NOGC_MUT_SLICE,
        Oracle::ExitOutStats(0, "101\n", 1, 1),
    );
    h.assert_measured("S3-UMS-nogc");
}

const STAGE3_I32_MUT_SLICE: &str = r#"
fn main() -> i64 {
    let mut v: Vec<i32> = vec[7919 as i32, 2 as i32, 3 as i32]
    let mut s: &mut [i32] = Vec::try_as_unique_mut_slice(v)
    s[0] = 101 as i32
    println(v[0])
    return 0
}
"#;

#[test]
fn stage3_unique_mut_slice_splices_non_i64_element_sizes() {
    let h = Harness::new();
    run_row(
        &h,
        "S3-UMS-i32",
        STAGE3_I32_MUT_SLICE,
        Oracle::ExitOutStats(0, "101\n", 1, 1),
    );
    h.assert_measured("S3-UMS-i32");
}

const STAGE3_OUT_OF_BOUNDS_MUT_SLICE: &str = r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    let mut s: &mut [i64] = Vec::try_as_unique_mut_slice(v)
    s[3] = 101
    println(v[0])
    return 0
}
"#;

#[test]
fn stage3_unique_mut_slice_view_stops_at_the_vec_length() {
    let h = Harness::new();
    run_row(
        &h,
        "S3-UMS-bound",
        STAGE3_OUT_OF_BOUNDS_MUT_SLICE,
        Oracle::ExitOut(134, ""),
    );
    h.assert_measured("S3-UMS-bound");
}

const STAGE3_EXCLUSIVITY_REJECTS: &[(&str, &str, &str)] = &[
    (
        "S3-UMS-alias-after",
        "E0713",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    let mut s: &mut [i64] = Vec::try_as_unique_mut_slice(v)
    let alias = v
    s[0] = 101
    println(alias[0])
    return 0
}
"#,
    ),
    (
        "S3-UMS-realloc",
        "E0711",
        r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    let mut s: &mut [i64] = Vec::try_as_unique_mut_slice(v)
    Vec::push(v, 4)
    s[0] = 101
    println(v[0])
    return 0
}
"#,
    ),
];

#[test]
fn stage3_unique_mut_slice_rejects_sharing_the_owner_after_the_view() {
    let h = Harness::new();
    run_rejects(&h, STAGE3_EXCLUSIVITY_REJECTS);
}

fn readme_claimed_runnable_example() -> String {
    let readme = fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../README.md"))
        .expect("README.md must be readable");
    let claim = readme
        .find("This compiles and runs:")
        .expect("README must still carry the runnable boundary claim");
    // the fence marker also appears inside the html highlighting comment above it
    let open = readme[claim..]
        .find("\n```rust\n")
        .map(|at| claim + at + "\n```rust\n".len())
        .expect("the claim must be followed by a fenced example");
    let close = readme[open..]
        .find("\n```")
        .map(|at| open + at)
        .expect("the fenced example must be closed");
    readme[open..close].to_string()
}

#[test]
fn readme_boundary_example_compiles_and_runs_as_claimed() {
    let h = Harness::new();
    let program = readme_claimed_runnable_example();
    run_row(&h, "README-boundary", &program, Oracle::ExitOut(0, "6\n"));
    h.assert_measured("README-boundary");
}

const STAGE3_GAP_FORMS: &[(&str, &str, &str)] = &[
    (
        "S3-UMS-gap-struct",
        "E0410",
        r#"
struct Box { v: Vec<i64> }
fn main() -> i64 {
    let mut b = Box{v: vec[7919, 2, 3]}
    let mut s: &mut [i64] = Vec::try_as_unique_mut_slice(b.v)
    s[0] = 101
    return 0
}
"#,
    ),
    (
        "S3-UMS-gap-rc",
        "E0412",
        r#"
fn main() -> i64 {
    let r = Rc::new(vec[7919, 2, 3])
    let mut s: &mut [i64] = Vec::try_as_unique_mut_slice(Rc::get(r))
    s[0] = 101
    return 0
}
"#,
    ),
    (
        "S3-UMS-gap-nested",
        "E0412",
        r#"
fn main() -> i64 {
    let mut v: Vec<Vec<i64>> = vec[vec[7919, 2, 3]]
    let mut s: &mut [i64] = Vec::try_as_unique_mut_slice(v[0])
    s[0] = 101
    return 0
}
"#,
    ),
];

#[test]
fn stage3_unique_mut_slice_gap_stays_closed_by_the_vec_surface() {
    let h = Harness::new();
    for (id, code, src) in STAGE3_GAP_FORMS {
        for (tag, opt) in LEVELS {
            let rendered = h.reject(id, tag, src, *opt);
            assert!(
                rendered.contains(&format!("[{code}]")),
                "{id} at {tag} must stay rejected with {code}. sema accepts a wider set of \
                 receivers than the BIR place_of that records the mutable loan, and BIR falls \
                 back to an aggregate instead of failing, so the day this form compiles it \
                 compiles with no static exclusivity at all. Give BIR a fail-closed path before \
                 opening it.\n{rendered}"
            );
        }
    }
}

const STAGE3_SHARED_REF_MUT_SLICE: &str = r#"
fn view(r: &Vec<i64>) -> &mut [i64] {
    return Vec::try_as_unique_mut_slice(*r)
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    let mut s: &mut [i64] = view(&v)
    return 0
}
"#;

const STAGE3_NONPLACE_MUT_SLICE: &str = r#"
fn main() -> i64 {
    let mut s: &mut [i64] = Vec::try_as_unique_mut_slice(1)
    return 0
}
"#;

#[test]
fn stage3_unique_mut_slice_runtime_and_splice_guards_are_present() {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let core = fs::read_to_string(manifest.join("../core/src/aelys_core_common.c"))
        .expect("core runtime source");
    let calls = fs::read_to_string(manifest.join("../codegen/src/lowering/calls.rs"))
        .expect("call lowering source");
    assert!(core.contains("__aelys_vec_try_as_unique_mut_slice"));
    assert!(core.contains("__aelys_rc_refcount"));
    assert!(core.contains("== 1"));
    assert!(calls.contains("__aelys_vec_try_as_unique_mut_slice"));
    assert!(calls.contains("AirType::Slice"));
}

const STAGE6_DEFAULT_R1: &str = r#"
nogc fn read(v: &Vec<i64>) -> i64 {
    let s: &[i64] = Vec::as_slice(*v)
    return Vec::len(*v) + s[0] - 7919
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7919, 2, 3]
    println(read(&v))
    println(v[0])
    return 0
}
"#;

const STAGE6_DEFAULT_R2: &str = r#"
struct Vec3 { x: i64, y: i64, z: i64 }
nogc fn compute_normals(vertices: &[Vec3], normals: &mut [Vec3]) -> i64 {
    normals[0].x = vertices[0].y
    return 0
}
fn main() -> i64 {
    let mut vertices: Vec<Vec3> = vec[Vec3 { x: 7919, y: 101, z: 3 }]
    let mut normals: Vec<Vec3> = vec[Vec3 { x: 0, y: 0, z: 0 }]
    let src: &[Vec3] = Vec::as_slice(vertices)
    let dst: &mut [Vec3] = Vec::try_as_unique_mut_slice(normals)
    compute_normals(src, dst)
    println(normals[0].x)
    println(vertices[0].x)
    return 0
}
"#;

const STAGE6_DEFAULT_R3: &str = r#"
nogc fn peek(v: &Vec<i64>) -> i64 { return (*v)[0] }
fn main() -> i64 {
    let v: Vec<i64> = vec[7919, 2, 3]
    println(peek(&v))
    return 0
}
"#;

const STAGE6_DEFAULT_KEEP_CALLBACK: &str = r#"
nogc fn invoke(x: i64) -> i64 { return x + 1 }
fn main() -> i64 { println(invoke(1)); return 0 }
"#;

const STAGE6_DEFAULT_REJECTS: &[(&str, &str, &str)] = &[
    (
        "SI-NG-managed",
        "E0727",
        r#"
nogc fn bad(v: &mut Vec<i64>) -> i64 {
    Vec::push(*v, 101)
    return 0
}
fn main() -> i64 { return 0 }
"#,
    ),
    (
        "SI-NG-callback",
        "E0727",
        r#"
nogc fn invoke(f: fn(i64) -> i64, x: i64) -> i64 { return f(x) }
fn inc(x: i64) -> i64 { return x + 1 }
fn main() -> i64 { return invoke(inc, 1) }
"#,
    ),
    (
        "SI-NG-by-value",
        "E0727",
        r#"
nogc fn owns(v: Vec<i64>) -> i64 { return Vec::len(v) }
fn main() -> i64 { return 0 }
"#,
    ),
];

#[test]
fn stage6_managed_nogc_boundary_is_on_the_default_invariant_path() {
    let h = Harness::new();
    run_row(
        &h,
        "SI-NG-read",
        STAGE6_DEFAULT_R1,
        Oracle::ExitOutStats(0, "3\n7919\n", 1, 1),
    );
    run_row(
        &h,
        "SI-NG-compute-normals",
        STAGE6_DEFAULT_R2,
        Oracle::ExitOutStats(0, "101\n7919\n", 2, 2),
    );
    run_row(
        &h,
        "SI-NG-shared-index",
        STAGE6_DEFAULT_R3,
        Oracle::ExitOutStats(0, "7919\n", 1, 1),
    );
    run_row(
        &h,
        "SI-NG-kept-callback",
        STAGE6_DEFAULT_KEEP_CALLBACK,
        Oracle::ExitOutStats(0, "2\n", 0, 0),
    );
    run_rejects(&h, STAGE6_DEFAULT_REJECTS);
    h.assert_measured("SI-NG-boundary");
}

#[test]
fn stage3_unique_mut_slice_keeps_static_exclusivity_fences() {
    let h = Harness::new();
    for (tag, opt) in LEVELS {
        let shared = h.reject("S3-UMS-shared-ref", tag, STAGE3_SHARED_REF_MUT_SLICE, *opt);
        assert!(
            shared.contains("E0422"),
            "shared receiver must keep E0422: {shared}"
        );
        let nonplace = h.reject("S3-UMS-nonplace", tag, STAGE3_NONPLACE_MUT_SLICE, *opt);
        assert!(
            nonplace.contains("E0421"),
            "non-place receiver must keep E0421: {nonplace}"
        );
        assert!(
            nonplace.contains("E0301"),
            "wrong receiver type must keep E0301: {nonplace}"
        );
    }
}


type ModuleFiles = &'static [(&'static str, &'static str)];

impl Harness {
    fn write_modules(&self, id: &str, tag: &str, files: ModuleFiles) -> PathBuf {
        let dir = self.dir.path().join(slug(id, tag));
        for (name, body) in files {
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).expect("create module directory");
            }
            fs::write(&path, body).expect("write module fixture");
        }
        dir.join("root.aelys")
    }

    fn compile_modules(
        &self,
        id: &str,
        tag: &str,
        files: ModuleFiles,
        opt: OptimizationLevel,
    ) -> Option<PathBuf> {
        let path = self.write_modules(id, tag, files);
        if let Err(err) = compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::Rc) {
            if linker_unavailable(&err.to_string()) {
                common::require_linker_skip(
                    "a skipped module row carries no runtime evidence at all",
                );
                self.linker_skips.set(self.linker_skips.get() + 1);
                return None;
            }
            panic!(
                "{id} at {tag} must compile:\n{}\nerror: {err}",
                render(files)
            );
        }
        let exe = exe_path_for(&path);
        assert!(
            exe.is_file(),
            "{id} at {tag}: compiled but produced no executable\n{}",
            render(files)
        );
        self.compiled_legs.set(self.compiled_legs.get() + 1);
        Some(exe)
    }

    fn reject_modules(
        &self,
        id: &str,
        tag: &str,
        files: ModuleFiles,
        opt: OptimizationLevel,
    ) -> String {
        let path = self.write_modules(id, tag, files);
        match lower_file_to_air(&path, opt) {
            Ok(_) => panic!(
                "{id} at {tag} must be rejected, but it was accepted:\n{}",
                render(files)
            ),
            Err(rendered) => rendered,
        }
    }
}

fn render(files: ModuleFiles) -> String {
    files
        .iter()
        .map(|(name, body)| format!("--- {name}\n{body}"))
        .collect::<Vec<_>>()
        .join("")
}

#[derive(Clone, Copy)]
enum ModOracle {
    ExitOut(i32, &'static str),
    Balanced(i32, &'static str),
    Stats(i32, &'static str, i64, i64),
}

fn run_module_row(h: &Harness, id: &str, files: ModuleFiles, oracle: ModOracle) {
    let (exit, out) = match oracle {
        ModOracle::ExitOut(e, o) | ModOracle::Balanced(e, o) | ModOracle::Stats(e, o, _, _) => {
            (e, o)
        }
    };
    for (name, opt) in LEVELS {
        let Some(exe) = h.compile_modules(id, name, files, *opt) else {
            eprintln!("{id}: linker unavailable, skipping");
            return;
        };
        for (alloc_name, alloc) in ALLOCATORS {
            let o = h.run(&exe, *alloc);
            assert_eq!(
                o.exit,
                exit,
                "{id} at {name} under {alloc_name}: the answer MUST be {exit}\n{}\nstdout: {:?}\nstderr:\n{}",
                render(files),
                o.stdout,
                o.stderr
            );
            assert_eq!(
                o.stdout,
                out,
                "{id} at {name} under {alloc_name}: stdout MUST be {out:?}\n{}",
                render(files)
            );
            let stats = || {
                o.stats.unwrap_or_else(|| {
                    panic!(
                        "{id} at {name}/{alloc_name}: no [rc] stats line\n{}",
                        render(files)
                    )
                })
            };
            match oracle {
                ModOracle::ExitOut(..) => {}
                ModOracle::Balanced(..) => {
                    let (allocs, frees) = stats();
                    assert_eq!(
                        allocs, frees,
                        "{id} at {name}/{alloc_name}: every buffer freed exactly once \
                         (allocs={allocs} frees={frees})"
                    );
                }
                ModOracle::Stats(_, _, a, f) => {
                    assert_eq!(
                        stats(),
                        (a, f),
                        "{id} at {name}/{alloc_name}: the exact pair MUST be ({a}, {f})"
                    );
                }
            }
        }
    }
}

fn run_module_rows(h: &Harness, rows: &[(&str, ModuleFiles, ModOracle)]) {
    let before = h.linker_skips.get();
    for (id, files, oracle) in rows {
        run_module_row(h, id, files, *oracle);
    }
    let skipped = h.linker_skips.get() - before;
    assert!(
        skipped == 0 || skipped == rows.len(),
        "{} of {} rows were skipped: the toolchain is either there or it is not",
        skipped,
        rows.len()
    );
}

fn run_module_rejects(h: &Harness, rows: &[(&str, &str, ModuleFiles)]) {
    for (id, code, files) in rows {
        for (name, opt) in REJECT_LEVELS {
            let rendered = h.reject_modules(id, name, files, *opt);
            assert!(
                rendered.contains(&format!("[{code}]")),
                "{id} at {name} MUST be rejected with {code}, got:\n{rendered}"
            );
        }
    }
}

const MOD_ROWS: &[(&str, ModuleFiles, ModOracle)] = &[
    (
        "SI-MOD-value",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.seven()\n}\n",
            ),
            ("m.aelys", "pub fn seven() -> i64 {\n    return 7\n}\n"),
        ],
        ModOracle::Stats(7, "", 0, 0),
    ),
    (
        "SI-MOD-string",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    println(m.greet())\n    return 0\n}\n",
            ),
            (
                "m.aelys",
                "pub fn greet() -> string {\n    return \"hello\"\n}\n",
            ),
        ],
        ModOracle::ExitOut(0, "hello\n"),
    ),
    (
        "SI-MOD-vec-across",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let xs = m.build(4)\n    return Vec::len(xs)\n}\n",
            ),
            (
                "m.aelys",
                "pub fn build(n: i64) -> vec<i64> {\n    let mut xs: vec<i64> = Vec::new()\n    Vec::push(xs, n)\n    return xs\n}\n",
            ),
        ],
        ModOracle::Balanced(1, ""),
    ),
    // nogc across a module boundary. this row cannot witness the nogc check itself, since its
    (
        "SI-MOD-nogc-across",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.clean(6) + m.plain(2)\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "m.aelys",
                "pub nogc fn clean(n: i64) -> i64 {\n    return n + 1\n}\n\npub fn plain(n: i64) -> i64 {\n    return n + 1\n}\n\npub fn unreached() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 0\n}\n",
            ),
        ],
        ModOracle::Stats(10, "", 0, 0),
    ),
    (
        "SI-MOD-struct",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let p = m.make(3, 4)\n    return p.a + p.b\n}\n",
            ),
            (
                "m.aelys",
                "pub struct P { pub a: i64, pub b: i64 }\n\npub fn make(x: i64, y: i64) -> P {\n    return P { a: x, b: y }\n}\n",
            ),
        ],
        ModOracle::Stats(7, "", 0, 0),
    ),
    (
        "SI-MOD-enum",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return match m.pick() {\n        m.T::Some(n) => n\n        m.T::None => 0\n    }\n}\n",
            ),
            (
                "m.aelys",
                "pub enum T { None, Some(i64) }\n\npub fn pick() -> T {\n    return T::Some(8)\n}\n",
            ),
        ],
        ModOracle::Stats(8, "", 0, 0),
    ),
    // two modules, one struct name, two layouts: the answer proves neither borrowed the other
    (
        "SI-MOD-same-name",
        &[
            (
                "root.aelys",
                "needs x\nneeds y\n\nfn main() -> i64 {\n    return x.make() + y.make()\n}\n",
            ),
            (
                "x.aelys",
                "struct Holder { v: i64 }\n\npub fn make() -> i64 {\n    let h = Holder { v: 10 }\n    return h.v\n}\n",
            ),
            (
                "y.aelys",
                "struct Holder { hi: i64, lo: i64 }\n\npub fn make() -> i64 {\n    let h = Holder { hi: 20, lo: 3 }\n    return h.hi + h.lo\n}\n",
            ),
        ],
        ModOracle::Stats(33, "", 0, 0),
    ),
    (
        "SI-MOD-lambdas",
        &[
            (
                "root.aelys",
                "needs p\nneeds q\n\nfn main() -> i64 {\n    return p.run() + q.run()\n}\n",
            ),
            (
                "p.aelys",
                "pub fn run() -> i64 {\n    let base: i64 = 10\n    let f = fn(a: i64) -> i64 { return a + base }\n    return f(1)\n}\n",
            ),
            (
                "q.aelys",
                "pub fn run() -> i64 {\n    let base: i64 = 20\n    let g = fn(a: i64) -> i64 { return a + base }\n    return g(1)\n}\n",
            ),
        ],
        ModOracle::ExitOut(32, ""),
    ),
    (
        "SI-MOD-nested-path",
        &[
            (
                "root.aelys",
                "needs app.util\nneeds app\n\nnogc fn pure() -> i64 {\n    return util.helper()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "util.aelys",
                "pub fn helper() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 1\n}\n",
            ),
            (
                "app.aelys",
                "needs util\n\npub fn go() -> i64 {\n    return util.helper()\n}\n",
            ),
            (
                "app/util.aelys",
                "pub nogc fn helper() -> i64 {\n    return 40\n}\n",
            ),
        ],
        ModOracle::Stats(40, "", 0, 0),
    ),
    (
        "SI-MOD-transitive",
        &[
            (
                "root.aelys",
                "needs mid\n\nfn main() -> i64 {\n    let p = mid.pass(41)\n    return mid.take(p)\n}\n",
            ),
            (
                "mid.aelys",
                "needs a\n\npub fn pass(x: i64) -> a.P {\n    return a.make(x)\n}\n\npub fn take(p: a.P) -> i64 {\n    return a.read(p)\n}\n",
            ),
            (
                "a.aelys",
                "pub struct P { pub v: i64 }\n\npub fn make(x: i64) -> P {\n    return P { v: x }\n}\n\npub fn read(p: P) -> i64 {\n    return p.v\n}\n",
            ),
        ],
        ModOracle::Stats(41, "", 0, 0),
    ),
];

const MOD_REJECTS: &[(&str, &str, ModuleFiles)] = &[
    (
        "SI-MOD-E0601",
        "E0601",
        &[(
            "root.aelys",
            "needs nope\n\nfn main() -> i64 {\n    return 0\n}\n",
        )],
    ),
    (
        "SI-MOD-E0602",
        "E0602",
        &[
            (
                "root.aelys",
                "needs a\n\nfn main() -> i64 {\n    return 0\n}\n",
            ),
            (
                "a.aelys",
                "needs b\n\npub fn f() -> i64 {\n    return 1\n}\n",
            ),
            (
                "b.aelys",
                "needs a\n\npub fn g() -> i64 {\n    return 2\n}\n",
            ),
        ],
    ),
    (
        "SI-MOD-E0603",
        "E0603",
        &[
            (
                "root.aelys",
                "needs m\nneeds n as m\n\nfn main() -> i64 {\n    return 0\n}\n",
            ),
            ("m.aelys", "pub fn f() -> i64 {\n    return 1\n}\n"),
            ("n.aelys", "pub fn g() -> i64 {\n    return 2\n}\n"),
        ],
    ),
    (
        "SI-MOD-E0604",
        "E0604",
        &[(
            "root.aelys",
            "needs __hidden\n\nfn main() -> i64 {\n    return 0\n}\n",
        )],
    ),
    (
        "SI-MOD-E0605",
        "E0605",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.secret()\n}\n",
            ),
            ("m.aelys", "fn secret() -> i64 {\n    return 1\n}\n"),
        ],
    ),
    (
        "SI-MOD-E0606",
        "E0606",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.absent()\n}\n",
            ),
            ("m.aelys", "pub fn seven() -> i64 {\n    return 7\n}\n"),
        ],
    ),
    (
        "SI-MOD-E0607",
        "E0607",
        &[(
            "root.aelys",
            "needs \"GL/glext.h\"\n\nfn main() -> i64 {\n    return 0\n}\n",
        )],
    ),
    (
        "SI-MOD-E0608",
        "E0608",
        &[
            (
                "root.aelys",
                "fn main() -> i64 {\n    return 0\n}\n\nneeds m\n",
            ),
            ("m.aelys", "pub fn f() -> i64 {\n    return 1\n}\n"),
        ],
    ),
    (
        "SI-MOD-E0609",
        "E0609",
        &[
            (
                "root.aelys",
                "needs m.*\n\nfn main() -> i64 {\n    return 0\n}\n",
            ),
            ("m.aelys", "pub fn f() -> i64 {\n    return 1\n}\n"),
        ],
    ),
    (
        "SI-MOD-E0610",
        "E0610",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.make(3).v\n}\n",
            ),
            (
                "m.aelys",
                "pub struct P { v: i64 }\n\npub fn make(n: i64) -> P {\n    return P { v: n }\n}\n",
            ),
        ],
    ),
    (
        "SI-MOD-E0611",
        "E0611",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let p = m.make(1)\n    return 0\n}\n",
            ),
            (
                "m.aelys",
                "struct P { v: i64 }\n\npub fn make(n: i64) -> P {\n    return P { v: n }\n}\n",
            ),
        ],
    ),
    (
        "SI-MOD-E0727",
        "E0727",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.allocates()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            (
                "m.aelys",
                "pub fn allocates() -> i64 {\n    let v: vec<i64> = Vec::new()\n    return 0\n}\n",
            ),
        ],
    ),
    (
        "SI-MOD-E0701",
        "E0701",
        &[
            (
                "root.aelys",
                "needs m\n\nfn leak() -> i64 {\n    let a = m.make(1)\n    let b = a\n    return a.id\n}\n\nfn main() -> i64 {\n    return leak()\n}\n",
            ),
            (
                "m.aelys",
                "pub struct Resource { pub id: i64 }\n\npub fn make(n: i64) -> Resource {\n    return Resource { id: n }\n}\n",
            ),
        ],
    ),
];

const MOD_KEPT: &[(&str, ModuleFiles, ModOracle)] = &[
    (
        "SI-MOD-E0601-kept",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.f()\n}\n",
            ),
            ("m.aelys", "pub fn f() -> i64 {\n    return 1\n}\n"),
        ],
        ModOracle::Stats(1, "", 0, 0),
    ),
    (
        "SI-MOD-E0603-kept",
        &[
            (
                "root.aelys",
                "needs m\nneeds n as k\n\nfn main() -> i64 {\n    return m.f() + k.g()\n}\n",
            ),
            ("m.aelys", "pub fn f() -> i64 {\n    return 1\n}\n"),
            ("n.aelys", "pub fn g() -> i64 {\n    return 2\n}\n"),
        ],
        ModOracle::Stats(3, "", 0, 0),
    ),
    (
        "SI-MOD-E0605-kept",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.shown()\n}\n",
            ),
            ("m.aelys", "pub fn shown() -> i64 {\n    return 1\n}\n"),
        ],
        ModOracle::Stats(1, "", 0, 0),
    ),
    (
        "SI-MOD-E0610-kept",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.make(3).v\n}\n",
            ),
            (
                "m.aelys",
                "pub struct P { pub v: i64 }\n\npub fn make(n: i64) -> P {\n    return P { v: n }\n}\n",
            ),
        ],
        ModOracle::Stats(3, "", 0, 0),
    ),
    (
        "SI-MOD-E0611-kept",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.make(1).v\n}\n",
            ),
            (
                "m.aelys",
                "pub struct P { pub v: i64 }\n\npub fn make(n: i64) -> P {\n    return P { v: n }\n}\n",
            ),
        ],
        ModOracle::Stats(1, "", 0, 0),
    ),
    (
        "SI-MOD-E0727-kept",
        &[
            (
                "root.aelys",
                "needs m\n\nnogc fn pure() -> i64 {\n    return m.clean()\n}\n\nfn main() -> i64 {\n    return pure()\n}\n",
            ),
            ("m.aelys", "pub fn clean() -> i64 {\n    return 4\n}\n"),
        ],
        ModOracle::Stats(4, "", 0, 0),
    ),
    (
        "SI-MOD-E0701-kept",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    let a = m.make(9)\n    let b = a\n    return b.id\n}\n",
            ),
            (
                "m.aelys",
                "pub struct Resource { pub id: i64 }\n\npub fn make(n: i64) -> Resource {\n    return Resource { id: n }\n}\n",
            ),
        ],
        ModOracle::Stats(9, "9\n", 0, 0),
    ),
    (
        "SI-MOD-E0602-kept",
        &[
            (
                "root.aelys",
                "needs a\n\nfn main() -> i64 {\n    return a.f()\n}\n",
            ),
            (
                "a.aelys",
                "needs b\n\npub fn f() -> i64 {\n    return b.g()\n}\n",
            ),
            ("b.aelys", "pub fn g() -> i64 {\n    return 2\n}\n"),
        ],
        ModOracle::Stats(2, "", 0, 0),
    ),
    (
        "SI-MOD-E0604-kept",
        &[
            (
                "root.aelys",
                "needs hidden\n\nfn main() -> i64 {\n    return hidden.f()\n}\n",
            ),
            ("hidden.aelys", "pub fn f() -> i64 {\n    return 1\n}\n"),
        ],
        ModOracle::Stats(1, "", 0, 0),
    ),
    (
        "SI-MOD-E0606-kept",
        &[
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.present()\n}\n",
            ),
            ("m.aelys", "pub fn present() -> i64 {\n    return 7\n}\n"),
        ],
        ModOracle::Stats(7, "", 0, 0),
    ),
    (
        "SI-MOD-E0607-kept",
        &[
            (
                "root.aelys",
                "needs gl\n\nfn main() -> i64 {\n    return gl.f()\n}\n",
            ),
            ("gl.aelys", "pub fn f() -> i64 {\n    return 1\n}\n"),
        ],
        ModOracle::Stats(1, "", 0, 0),
    ),
    (
        "SI-MOD-E0608-kept",
        &[
            (
                "root.aelys",
                "needs m\nneeds n\n\nfn main() -> i64 {\n    return m.f() + n.g()\n}\n",
            ),
            ("m.aelys", "pub fn f() -> i64 {\n    return 1\n}\n"),
            ("n.aelys", "pub fn g() -> i64 {\n    return 5\n}\n"),
        ],
        ModOracle::Stats(6, "", 0, 0),
    ),
    (
        "SI-MOD-E0609-kept",
        &[
            (
                "root.aelys",
                "needs f from m\n\nfn main() -> i64 {\n    return f()\n}\n",
            ),
            ("m.aelys", "pub fn f() -> i64 {\n    return 1\n}\n"),
        ],
        ModOracle::Stats(1, "", 0, 0),
    ),
];

#[test]
fn group_mod_multi_file_programs_run_and_account_for_their_memory() {
    let h = Harness::new();
    run_module_rows(&h, MOD_ROWS);
    h.assert_measured("group_mod");
}

#[test]
fn group_mod_every_rejection_has_a_kept_twin() {
    let h = Harness::new();
    run_module_rejects(&h, MOD_REJECTS);
    run_module_rows(&h, MOD_KEPT);
    h.assert_measured("group_mod_kept");
}

const FFI_LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const FFI_HEAD: &str =
    "unsafe extern fn ffsl(x: i64) -> i64\n\nfn main() -> i64 {\n    unsafe { return ffsl(1024) }\n}\n";

const FFI_AUDITED_NOGC: &str = "unsafe extern nogc fn ffsl(x: i64) -> i64\n\nnogc fn probe(x: i64) -> i64 {\n    unsafe { return ffsl(x) }\n}\n\nfn main() -> i64 {\n    return probe(1024)\n}\n";

const FFI_TWIN_FFSL: &str = "unsafe extern fn malloc(n: i64) -> i64\nunsafe extern fn ffsl(x: i64) -> i64\n\nfn main() -> i64 {\n    unsafe { return ffsl(1024) }\n}\n";

const FFI_TWIN_LABS: &str =
    "unsafe extern fn labs(x: i64) -> i64\n\nfn main() -> i64 {\n    unsafe { return labs(-7) }\n}\n";

fn ffi_run_everywhere(h: &Harness, id: &str, src: &str, exit: i32, allocs: i64, frees: i64) {
    for (level, opt) in FFI_LEVELS {
        let Some(exe) = h.compile(id, level, src, *opt) else {
            eprintln!("{id}: linker unavailable, skipping");
            return;
        };
        for (alloc_name, alloc) in ALLOCATORS {
            let o = h.run(&exe, *alloc);
            assert_eq!(
                o.exit, exit,
                "{id} at {level} under {alloc_name}: the answer MUST be {exit}\n{src}\nstderr:\n{}",
                o.stderr
            );
            assert_eq!(o.stdout, "", "{id} at {level} under {alloc_name}: stdout");
            let stats = o.stats.unwrap_or_else(|| {
                panic!("{id} at {level}/{alloc_name}: no [rc] stats line\nstderr:\n{}", o.stderr)
            });
            assert_eq!(
                stats,
                (allocs, frees),
                "{id} at {level}/{alloc_name}: the exact pair MUST be ({allocs}, {frees})"
            );
        }
    }
}

#[test]
fn group_ffi_aelys_calls_c_at_four_levels_under_both_allocators() {
    let h = Harness::new();
    ffi_run_everywhere(&h, "SI-FFI-HEAD", FFI_HEAD, 11, 0, 0);
    h.assert_measured("group_ffi_head");
}

// (id, code, the refused spelling, the twin, the twin's answer)
const FFI_PAIRS: &[(&str, &str, &str, &str, i32)] = &[
    (
        "SI-FFI-E0613-decl",
        "E0613",
        "unsafe extern fn aelys_immix_alloc(n: i64) -> i64\n\nfn main() -> i64 {\n    return 0\n}\n",
        FFI_TWIN_FFSL,
        11,
    ),
    (
        "SI-FFI-E0613-def",
        "E0613",
        "fn malloc(n: i64) -> i64 {\n    return n\n}\n\nfn main() -> i64 {\n    return malloc(3)\n}\n",
        "fn mallocate(n: i64) -> i64 {\n    return n\n}\n\nfn main() -> i64 {\n    return mallocate(3)\n}\n",
        3,
    ),
    (
        "SI-FFI-E0614-pub",
        "E0614",
        "pub unsafe extern fn ffsl(x: i64) -> i64\n\nfn main() -> i64 {\n    return 0\n}\n",
        FFI_HEAD,
        11,
    ),
    (
        "SI-FFI-E0614-safe",
        "E0614",
        "extern fn ffsl(x: i64) -> i64\n\nfn main() -> i64 {\n    return 0\n}\n",
        FFI_HEAD,
        11,
    ),
    (
        "SI-FFI-E0614-body",
        "E0614",
        "unsafe extern fn ffsl(x: i64) -> i64 { return x }\n\nfn main() -> i64 {\n    return 0\n}\n",
        FFI_HEAD,
        11,
    ),
    (
        "SI-FFI-E0615-string",
        "E0615",
        "unsafe extern fn f(x: string) -> i64\n\nfn main() -> i64 {\n    return 0\n}\n",
        FFI_HEAD,
        11,
    ),
    // the verdict is on the type and not on how it is spelled
    (
        "SI-FFI-E0615-case",
        "E0615",
        "unsafe extern fn f(x: sTRING) -> i64\n\nfn main() -> i64 {\n    return 0\n}\n",
        FFI_HEAD,
        11,
    ),
    (
        "SI-FFI-E0615-return-ref",
        "E0615",
        "unsafe extern fn f(x: i64) -> &i64\n\nfn main() -> i64 {\n    return 0\n}\n",
        "unsafe extern fn takes_ref(x: &i64) -> i64\nunsafe extern fn ffsl(x: i64) -> i64\n\nfn main() -> i64 {\n    unsafe { return ffsl(1024) }\n}\n",
        11,
    ),
    (
        "SI-FFI-E0616-let",
        "E0616",
        "unsafe extern fn labs(x: i64) -> i64\n\nfn main() -> i64 {\n    let g = labs\n    return 0\n}\n",
        FFI_TWIN_LABS,
        7,
    ),
    (
        "SI-FFI-E0616-argument",
        "E0616",
        "unsafe extern fn labs(x: i64) -> i64\n\nfn apply(g: fn(i64) -> i64) -> i64 {\n    return g(-7)\n}\n\nfn main() -> i64 {\n    return apply(labs)\n}\n",
        "fn neg(x: i64) -> i64 {\n    return 0 - x\n}\n\nfn apply(g: fn(i64) -> i64) -> i64 {\n    return g(-7)\n}\n\nfn main() -> i64 {\n    return apply(neg)\n}\n",
        7,
    ),
    (
        "SI-FFI-E0617-bare",
        "E0617",
        "unsafe extern fn labs(x: i64) -> i64\n\nfn main() -> i64 {\n    return labs(-7)\n}\n",
        FFI_TWIN_LABS,
        7,
    ),
    (
        "SI-FFI-E0617-condition",
        "E0617",
        "unsafe extern fn labs(x: i64) -> i64\n\nfn main() -> i64 {\n    if labs(-7) > 0 { return 1 }\n    return 0\n}\n",
        "unsafe extern fn labs(x: i64) -> i64\n\nfn main() -> i64 {\n    if unsafe { labs(-7) } > 0 { return 1 }\n    return 0\n}\n",
        1,
    ),
];

const FFI_E0612_REFUSED: ModuleFiles = &[
    (
        "root.aelys",
        "needs a\nneeds b\n\nfn main() -> i64 {\n    return a.use_a(-20) + b.use_b(-17)\n}\n",
    ),
    (
        "a.aelys",
        "unsafe extern fn labs(x: i64) -> i64\n\npub fn use_a(x: i64) -> i64 {\n    unsafe { return labs(x) }\n}\n",
    ),
    (
        "b.aelys",
        "unsafe extern nogc fn labs(x: i64) -> i64\n\npub fn use_b(x: i64) -> i64 {\n    unsafe { return labs(x) }\n}\n",
    ),
];

const FFI_E0612_TWIN: ModuleFiles = &[
    (
        "root.aelys",
        "needs a\nneeds b\n\nfn main() -> i64 {\n    return a.use_a(-20) + b.use_b(-17)\n}\n",
    ),
    (
        "a.aelys",
        "unsafe extern fn labs(x: i64) -> i64\n\npub fn use_a(x: i64) -> i64 {\n    unsafe { return labs(x) }\n}\n",
    ),
    (
        "b.aelys",
        "unsafe extern fn labs(x: i64) -> i64\n\npub fn use_b(x: i64) -> i64 {\n    unsafe { return labs(x) }\n}\n",
    ),
];

const FFI_LIB_C: &str = "long aelys_si_ffi_triple(long x) { return 3 * x; }\n";

const FFI_LIB_AE: &str = "unsafe extern fn aelys_si_ffi_triple(x: i64) -> i64\n\nfn main() -> i64 {\n    unsafe { return aelys_si_ffi_triple(14) }\n}\n";

// the allocating shape, so a member defining a reserved symbol is actually pulled in
const FFI_LIB_ALLOCATING_AE: &str = "unsafe extern fn aelys_si_ffi_triple(x: i64) -> i64\n\nfn main() -> i64 {\n    let mut v = Vec::new()\n    Vec::push(v, 11)\n    unsafe { return aelys_si_ffi_triple(14) }\n}\n";

const FFI_HOSTILE_MEMCPY_C: &str = "#include <stddef.h>\nvoid *memcpy(void *d, const void *s, size_t n) {\n    unsigned char *a = d; const unsigned char *b = s;\n    while (n--) *a++ = *b++;\n    return d;\n}\n";

// a missing toolchain must redden the row, never skip it
fn ffi_tool(name: &str) -> String {
    let found = Command::new(name).arg("--version").output();
    assert!(
        found.map(|out| out.status.success()).unwrap_or(false),
        "group_ffi builds a real library, so `{name}` is required and its absence is a failure"
    );
    name.to_string()
}

fn ffi_tool_run(program: &str, args: &[&str], dir: &Path) -> std::process::Output {
    Command::new(program)
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|err| panic!("failed to run `{program}`: {err}"))
}

fn ffi_object(dir: &Path, source: &str, body: &str) {
    fs::write(dir.join(source), body).expect("write c source");
    let object = source.replace(".c", ".o");
    let out = ffi_tool_run(&ffi_tool("clang"), &["-fPIC", "-c", "-o", &object, source], dir);
    assert!(out.status.success(), "clang failed: {out:?}");
}

fn ffi_archive(dir: &Path, name: &str, objects: &[&str]) {
    fs::create_dir_all(dir.join("lib")).expect("lib dir");
    let path = format!("lib/lib{name}.a");
    let mut args = vec!["rcs", path.as_str()];
    args.extend_from_slice(objects);
    let out = ffi_tool_run(&ffi_tool("ar"), &args, dir);
    assert!(out.status.success(), "ar failed: {out:?}");
}

fn ffi_link_to(dir: &Path, libraries: &[&str]) -> LinkRequirement {
    LinkRequirement {
        search_paths: vec![dir.join("lib")],
        libraries: libraries.iter().map(|name| (*name).to_string()).collect(),
    }
}

fn ffi_compile_linked(
    dir: &Path,
    stem: &str,
    source: &str,
    link: &LinkRequirement,
    opt: OptimizationLevel,
) -> Result<PathBuf, String> {
    let path = dir.join(format!("{stem}.aelys"));
    fs::write(&path, source).expect("write aelys source");
    compile_file_with_llvm_linked(&path, opt, false, RuntimeVariant::Rc, link)
        .map(|_| dir.join(stem))
        .map_err(|err| err.to_string())
}

#[test]
fn group_ffi_a_third_party_archive_built_by_this_row_links_and_answers_forty_two() {
    let dir = tempdir().expect("tempdir");
    ffi_object(dir.path(), "sitriple.c", FFI_LIB_C);
    ffi_archive(dir.path(), "sifftriple", &["sitriple.o"]);
    let link = ffi_link_to(dir.path(), &["sifftriple"]);
    for (level, opt) in FFI_LEVELS {
        let exe = ffi_compile_linked(dir.path(), "sil1", FFI_LIB_AE, &link, *opt)
            .unwrap_or_else(|err| panic!("SI-FFI-LIB at {level}: the link MUST succeed: {err}"));
        let out = Command::new(&exe).output().expect("run the linked program");
        assert_eq!(
            exit_code(&out.status),
            42,
            "SI-FFI-LIB at {level}: 3 x 14 is the answer no accident gives"
        );
    }
}

fn ffi_reject_everywhere(h: &Harness, id: &str, code: &str, src: &str) {
    for (level, opt) in FFI_LEVELS {
        let rendered = h.reject(id, level, src, *opt);
        assert!(
            rendered.contains(&format!("[{code}]")),
            "{id} at {level} MUST be rejected with {code}, got:\n{rendered}"
        );
    }
}

#[test]
fn group_ffi_every_rejection_has_a_kept_twin() {
    let h = Harness::new();
    let mut codes: HashSet<&str> = HashSet::new();
    for (id, code, refused, twin, answer) in FFI_PAIRS {
        ffi_reject_everywhere(&h, id, code, refused);
        ffi_run_everywhere(&h, id, twin, *answer, 0, 0);
        codes.insert(code);
    }

    for (level, opt) in FFI_LEVELS {
        let rendered = h.reject_modules("SI-FFI-E0612", level, FFI_E0612_REFUSED, *opt);
        assert!(
            rendered.contains("[E0612]") && rendered.contains("nogc"),
            "SI-FFI-E0612 at {level} MUST be rejected with E0612 naming the claim, got:\n{rendered}"
        );
    }
    run_module_row(
        &h,
        "SI-FFI-E0612-kept",
        FFI_E0612_TWIN,
        ModOracle::Stats(37, "", 0, 0),
    );
    codes.insert("E0612");

    let dir = tempdir().expect("tempdir");
    ffi_object(dir.path(), "sitriple.c", FFI_LIB_C);
    ffi_archive(dir.path(), "sifftriple", &["sitriple.o"]);
    ffi_object(dir.path(), "simemcpy.c", FFI_HOSTILE_MEMCPY_C);
    ffi_archive(dir.path(), "sihostile", &["simemcpy.o"]);
    let hostile = ffi_link_to(dir.path(), &["sifftriple", "sihostile"]);
    let err = ffi_compile_linked(
        dir.path(),
        "si618",
        FFI_LIB_ALLOCATING_AE,
        &hostile,
        OptimizationLevel::None,
    )
    .expect_err("SI-FFI-E0618: memcpy is one of the sixteen the runtime imports");
    assert!(
        err.contains("E0618") && err.contains("memcpy"),
        "SI-FFI-E0618: the verdict MUST name the symbol, got:\n{err}"
    );
    assert!(
        !dir.path().join("si618").exists(),
        "SI-FFI-E0618: no executable may survive the rejection"
    );
    let clean = ffi_link_to(dir.path(), &["sifftriple"]);
    let exe = ffi_compile_linked(
        dir.path(),
        "si618k",
        FFI_LIB_ALLOCATING_AE,
        &clean,
        OptimizationLevel::None,
    )
    .unwrap_or_else(|err| panic!("SI-FFI-E0618-kept: the same program MUST link: {err}"));
    let out = Command::new(&exe).output().expect("run the linked program");
    assert_eq!(
        exit_code(&out.status),
        42,
        "SI-FFI-E0618-kept: the same program, one library short of the claim"
    );
    codes.insert("E0618");

    let mut seen: Vec<&str> = codes.into_iter().collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        vec!["E0612", "E0613", "E0614", "E0615", "E0616", "E0617", "E0618"],
        "group_ffi: every code the run introduced MUST hold a pair here"
    );
    h.assert_measured("group_ffi_kept");
}

fn ffi_declared(symbol: &str, declared_nogc: bool) -> BirExtern {
    BirExtern {
        foreign: ForeignDecl {
            symbol: symbol.to_string(),
            calling_conv: ForeignConv::C,
            is_unsafe: true,
            span: Span::dummy(),
        },
        declared_nogc,
    }
}

const FFI_SIX: [Effect; 6] = [
    Effect::Managed,
    Effect::Alloc,
    Effect::Panic,
    Effect::Unwind,
    Effect::Block,
    Effect::Io,
];

fn ffi_call(callee: Option<&str>) -> BirStmt {
    BirStmt {
        kind: BirStmtKind::Assign {
            dest: BirPlace {
                local: BirLocalId(0),
                proj: Vec::new(),
            },
            rvalue: BirRvalue::Call {
                callee: callee.map(str::to_string),
                args: vec![BirOperand::Const],
                indirect_nogc: false,
            },
        },
        span: Span::dummy(),
    }
}

fn ffi_body(name: &str, stmts: Vec<BirStmt>) -> BirBody {
    BirBody {
        name: name.to_string(),
        locals: Vec::new(),
        arg_count: 0,
        blocks: vec![BirBlock {
            id: BirBlockId(0),
            stmts,
            term: BirTerminator::Return(None),
            term_span: Span::dummy(),
        }],
        entry: BirBlockId(0),
        span: Span::dummy(),
        scope_exits: Vec::new(),
        returns: Vec::new(),
        reassigns: Vec::new(),
        is_toplevel: false,
        return_type: InferType::I64,
        build_errors: Vec::new(),
        scope_deaths: Vec::new(),
        intrinsic_effects: EffectSet::EMPTY,
        managed_witness: None,
        declared_nogc: false,
    }
}

fn ffi_summaries() -> std::collections::HashMap<String, EffectSet> {
    let mut externs = std::collections::HashMap::new();
    externs.insert("c_ext".to_string(), ffi_declared("c_ext", false));
    externs.insert("c_ext_nogc".to_string(), ffi_declared("c_ext_nogc", true));
    let program = BirProgram {
        bodies: vec![
            ffi_body("caller_of_unknown", vec![ffi_call(Some("c_absent"))]),
            ffi_body("caller_of_known", vec![ffi_call(Some("leaf"))]),
            ffi_body("leaf", Vec::new()),
            ffi_body("caller_of_extern", vec![ffi_call(Some("c_ext"))]),
            ffi_body("caller_of_nogc_extern", vec![ffi_call(Some("c_ext_nogc"))]),
        ],
        externs,
    };
    effect_summaries_with_imports(&program, &std::collections::HashMap::new())
}

fn ffi_pin(summaries: &std::collections::HashMap<String, EffectSet>, name: &str, want: [bool; 6]) -> EffectSet {
    let eff = *summaries
        .get(name)
        .unwrap_or_else(|| panic!("no summary for `{name}`"));
    for (effect, expected) in FFI_SIX.iter().zip(want) {
        assert_eq!(
            eff.contains(*effect),
            expected,
            "{name}: {effect:?} should be {expected}"
        );
    }
    eff
}

#[test]
fn group_ffi_the_inherited_barrier_stays_readable_from_here() {
    let summaries = ffi_summaries();
    let barrier = ffi_pin(&summaries, "caller_of_unknown", [true, true, true, false, false, false]);
    ffi_pin(&summaries, "caller_of_known", [false; 6]);
    assert_ne!(
        barrier,
        summaries["caller_of_known"],
        "an absent callee and a known one must not summarise alike"
    );
    assert_ne!(
        barrier, EXTERN_DEFAULT,
        "the barrier and the declared default must stay distinguishable"
    );
}

#[test]
fn group_ffi_the_extern_defaults_are_six_bits_and_the_audited_nogc_claim_allocates_nothing() {
    let summaries = ffi_summaries();
    let plain = ffi_pin(&summaries, "caller_of_extern", [true; 6]);
    assert_eq!(plain, EXTERN_DEFAULT, "the plain default is the six bits");
    let nogc = ffi_pin(
        &summaries,
        "caller_of_nogc_extern",
        [false, true, true, true, true, true],
    );
    assert_eq!(nogc, EXTERN_NOGC, "the nogc default is derived, not carried");
    assert!(nogc.is_nogc(), "a nogc extern leaves its caller nogc");
    assert!(
        nogc.contains(Effect::Alloc),
        "nogc is managed free, it never promises the callee allocates nothing"
    );

    let h = Harness::new();
    ffi_run_everywhere(&h, "SI-FFI-NOGC", FFI_AUDITED_NOGC, 11, 0, 0);
    h.assert_measured("group_ffi_nogc");
}

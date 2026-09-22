// ! `ref` and dropped `mutable`, so the two were one type and a shared borrow could be bound to a

use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const ALLOCATORS: &[(&str, Option<&str>)] = &[("immix", None), ("malloc", Some("malloc"))];

static WARM: Once = Once::new();

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
}

struct Harness {
    dir: TempDir,
    legs: Cell<usize>,
    linker_skips: Cell<usize>,
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

fn linker_skip_declared() -> bool {
    std::env::var("AELYS_ALLOW_LINKER_SKIP").is_ok()
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
            legs: Cell::new(0),
            linker_skips: Cell::new(0),
        }
    }

    fn write(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let path = self.dir.path().join(format!("{}.aelys", slug(id, tag)));
        fs::write(&path, src).expect("write fixture");
        path
    }

    fn reject(&self, id: &str, tag: &str, src: &str, opt: OptimizationLevel) -> String {
        let path = self.write(id, tag, src);
        match lower_file_to_air(&path, opt) {
            Ok(_) => panic!("{id} at {tag} MUST be rejected, but it was accepted:\n{src}"),
            Err(rendered) => rendered,
        }
    }

    fn fenced_row(&self, id: &str, src: &str, code: &str) -> String {
        let mut last = String::new();
        for (tag, opt) in LEVELS {
            let rendered = self.reject(id, tag, src, *opt);
            self.legs.set(self.legs.get() + 1);
            assert!(
                rendered.contains(&format!("[{code}]")),
                "{id} at {tag} MUST be refused by {code} specifically\nsource:\n{src}\ngot:\n{rendered}"
            );
            last = rendered;
        }
        last
    }

    fn accepts(&self, id: &str, src: &str) {
        for (tag, opt) in LEVELS {
            let path = self.write(id, tag, src);
            self.legs.set(self.legs.get() + 1);
            if let Err(rendered) = lower_file_to_air(&path, *opt) {
                panic!("{id} at {tag} MUST be accepted:\nsource:\n{src}\ngot:\n{rendered}");
            }
        }
    }

    fn compile(&self, id: &str, tag: &str, src: &str, opt: OptimizationLevel) -> Option<PathBuf> {
        let path = self.write(id, tag, src);
        match compile_file_with_llvm_variant(&path, opt, false, RuntimeVariant::Rc) {
            Ok(()) => {}
            Err(err) => {
                if linker_unavailable(&err.to_string()) {
                    self.linker_skips.set(self.linker_skips.get() + 1);
                    return None;
                }
                panic!("{id} at {tag} must compile:\n{src}\nerror: {err}");
            }
        }
        let exe = exe_path_for(&path);
        exe.is_file().then_some(exe)
    }

    fn run(&self, exe: &Path, alloc: Option<&str>) -> Outcome {
        let mut cmd = Command::new(exe);
        cmd.env("AELYS_RC_STATS", "1");
        if let Some(a) = alloc {
            cmd.env("AELYS_ALLOC", a);
        }
        let out = cmd.output().expect("run compiled exe");
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        Outcome {
            exit: exit_code(&out.status),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stats: parse_stats(&stderr),
            stderr,
        }
    }

    fn value_row(&self, id: &str, src: &str, stdout: &str, allocs: i64, frees: i64) {
        for (tag, opt) in LEVELS {
            let Some(exe) = self.compile(id, tag, src, *opt) else {
                assert!(
                    linker_skip_declared(),
                    "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped value \
                     row carries no runtime evidence at all"
                );
                return;
            };
            for (alloc_name, alloc) in ALLOCATORS {
                let o = self.run(&exe, *alloc);
                self.legs.set(self.legs.get() + 1);
                assert_eq!(
                    o.exit, 0,
                    "{id} at {tag}/{alloc_name} must exit 0\nsource:\n{src}\nstderr:\n{}",
                    o.stderr
                );
                assert_eq!(
                    o.stdout, stdout,
                    "{id} at {tag}/{alloc_name}: stdout MUST be {stdout:?}\nsource:\n{src}"
                );
                let (a, f) = o.stats.unwrap_or_else(|| {
                    panic!(
                        "{id} at {tag}/{alloc_name}: no [rc] stats line\nstderr:\n{}",
                        o.stderr
                    )
                });
                assert_eq!(
                    (a, f),
                    (allocs, frees),
                    "{id} at {tag}/{alloc_name}: MUST be allocs={allocs} frees={frees}, got \
                     allocs={a} frees={f}\nsource:\n{src}"
                );
            }
        }
    }

    fn assert_legs(&self, expected: usize) {
        if self.linker_skips.get() > 0 && linker_skip_declared() {
            return;
        }
        assert_eq!(
            self.legs.get(),
            expected,
            "this test must execute exactly {expected} legs; a leg that silently stopped running \
             is the confident zero this run keeps hitting"
        );
    }
}

const R1_LET: &str = "fn main() -> i64 {\n\
                      \x20   let mut x: i64 = 7919\n\
                      \x20   let p: &mut i64 = &x\n\
                      \x20   *p = 101\n\
                      \x20   return x\n\
                      }\n";

const R2_ARG: &str = "fn w(p: &mut i64) -> i64 {\n\
                      \x20   *p = 101\n\
                      \x20   return 0\n\
                      }\n\
                      fn main() -> i64 {\n\
                      \x20   let mut x: i64 = 7919\n\
                      \x20   let q = w(&x)\n\
                      \x20   return x\n\
                      }\n";

const R3_RET: &str = "fn g(r: &i64) -> &mut i64 { return r }\n\
                      fn main() -> i64 { return 0 }\n";

const R4_ASSIGN: &str = "fn main() -> i64 {\n\
                         \x20   let mut x: i64 = 7919\n\
                         \x20   let mut y: i64 = 1\n\
                         \x20   let mut p: &mut i64 = &mut y\n\
                         \x20   p = &x\n\
                         \x20   *p = 101\n\
                         \x20   return x\n\
                         }\n";

const R5_IF_ARMS: &str = "fn main() -> i64 {\n\
                          \x20   let mut x: i64 = 7919\n\
                          \x20   let c: bool = true\n\
                          \x20   let p: &mut i64 = if c { &x } else { &x }\n\
                          \x20   *p = 101\n\
                          \x20   return x\n\
                          }\n";

const R6_BLOCK_TAIL: &str = "fn main() -> i64 {\n\
                             \x20   let mut x: i64 = 7919\n\
                             \x20   let p: &mut i64 = { &x }\n\
                             \x20   *p = 101\n\
                             \x20   return x\n\
                             }\n";

const ROUTES: &[(&str, &str)] = &[
    ("R1-annotated-let", R1_LET),
    ("R2-call-argument", R2_ARG),
    ("R3-return-type", R3_RET),
    ("R4-assign-to-existing-mut", R4_ASSIGN),
    ("R5-if-arms", R5_IF_ARMS),
    ("R6-block-tail", R6_BLOCK_TAIL),
];

#[test]
fn every_acquisition_route_is_refused_by_e0416() {
    let h = Harness::new();
    for (id, src) in ROUTES {
        h.fenced_row(id, src, "E0416");
    }
    h.assert_legs(ROUTES.len() * 4);
}


const P1_WHOLE_ELEMENT: &str = "nogc fn poke(r: &mut Vec<i64>) -> i64 {\n\
                                \x20   let p: &mut i64 = &(*r)[0]\n\
                                \x20   *p = 101\n\
                                \x20   return 0\n\
                                }\n\
                                fn main() -> i64 { return 0 }\n";

const P2_FIELD_OF_ELEMENT: &str = "struct S { n: i64 }\n\
                                   nogc fn poke(r: &mut Vec<S>) -> i64 {\n\
                                   \x20   let p: &mut i64 = &(*r)[0].n\n\
                                   \x20   *p = 101\n\
                                   \x20   return 0\n\
                                   }\n\
                                   fn main() -> i64 { return 0 }\n";

const P3_FIELD_ALONE: &str = "struct S { n: i64 }\n\
                              nogc fn poke(s: &mut S) -> i64 {\n\
                              \x20   let p: &mut i64 = &(*s).n\n\
                              \x20   *p = 101\n\
                              \x20   return 0\n\
                              }\n\
                              fn main() -> i64 { return 0 }\n";

const P4_VIA_LOCAL_SLICE: &str = "nogc fn poke(r: &mut Vec<i64>) -> i64 {\n\
                                  \x20   let s: &mut [i64] = (*r)[0..2]\n\
                                  \x20   let p: &mut i64 = &s[0]\n\
                                  \x20   *p = 101\n\
                                  \x20   return 0\n\
                                  }\n\
                                  fn main() -> i64 { return 0 }\n";

const P5_DYNAMIC_INDEX: &str = "nogc fn poke(r: &mut Vec<i64>, i: i64) -> i64 {\n\
                                \x20   let p: &mut i64 = &(*r)[i]\n\
                                \x20   *p = 101\n\
                                \x20   return 0\n\
                                }\n\
                                fn main() -> i64 { return 0 }\n";

const P6_CALL_THROUGH_PROJECTION: &str = "nogc fn w(p: &mut i64) -> i64 {\n\
                                          \x20   *p = 101\n\
                                          \x20   return 0\n\
                                          }\n\
                                          nogc fn poke(r: &mut Vec<i64>) -> i64 { return w(&(*r)[0]) }\n\
                                          fn main() -> i64 { return 0 }\n";

const PROJECTIONS: &[(&str, &str)] = &[
    ("P1-whole-element", P1_WHOLE_ELEMENT),
    ("P2-field-of-element", P2_FIELD_OF_ELEMENT),
    ("P3-field-alone", P3_FIELD_ALONE),
    ("P4-via-local-slice", P4_VIA_LOCAL_SLICE),
    ("P5-dynamic-index", P5_DYNAMIC_INDEX),
    ("P6-call-through-projection", P6_CALL_THROUGH_PROJECTION),
];

#[test]
fn every_projection_shape_is_refused_by_e0416() {
    let h = Harness::new();
    for (id, src) in PROJECTIONS {
        h.fenced_row(id, src, "E0416");
    }
    h.assert_legs(PROJECTIONS.len() * 4);
}

// the must-not-move controls. `&mut t` to `&t` drops write access and so can never create

const W1_WEAKEN_LET: &str = "fn main() -> i64 {\n\
                             \x20   let mut x: i64 = 7919\n\
                             \x20   let p: &i64 = &mut x\n\
                             \x20   println(*p)\n\
                             \x20   return 0\n\
                             }\n";

const W2_WEAKEN_ARG: &str = "fn rd(p: &i64) -> i64 { return *p }\n\
                             fn main() -> i64 {\n\
                             \x20   let mut x: i64 = 7919\n\
                             \x20   println(rd(&mut x))\n\
                             \x20   return 0\n\
                             }\n";

const W3_MUT_STILL_WRITES: &str = "fn main() -> i64 {\n\
                                   \x20   let mut x: i64 = 7919\n\
                                   \x20   let p: &mut i64 = &mut x\n\
                                   \x20   *p = 101\n\
                                   \x20   println(x)\n\
                                   \x20   return 0\n\
                                   }\n";

#[test]
fn the_safe_weakening_still_compiles_and_still_reads() {
    let h = Harness::new();
    h.value_row("W1-weaken-let", W1_WEAKEN_LET, "7919\n", 0, 0);
    h.value_row("W2-weaken-arg", W2_WEAKEN_ARG, "7919\n", 0, 0);
    h.value_row("W3-mut-still-writes", W3_MUT_STILL_WRITES, "101\n", 0, 0);
    h.assert_legs(3 * 8);
}

// routes must not catch this, or the closure is a ban on references into a vec rather than a
const A21_SHARED_READ: &str = "nogc fn peek(r: &mut Vec<i64>) -> i64 {\n\
                               \x20   let p: &i64 = &(*r)[0]\n\
                               \x20   return *p\n\
                               }\n\
                               fn main() -> i64 { return 0 }\n";

#[test]
fn the_shared_read_into_a_vec_is_not_caught() {
    let h = Harness::new();
    h.accepts("A21-shared-read", A21_SHARED_READ);
    h.assert_legs(4);
}

// shared and printed `101|101` at allocs=1, where the direct spelling of the same store detaches

const DIRECT_SPELLING: &str = "fn poke(r: &mut Vec<i64>) -> i64 {\n\
                               \x20   (*r)[0] = 101\n\
                               \x20   return 0\n\
                               }\n\
                               fn main() -> i64 {\n\
                               \x20   let mut v: Vec<i64> = vec[7919, 2, 3]\n\
                               \x20   let w: Vec<i64> = v\n\
                               \x20   let q = poke(&mut v)\n\
                               \x20   println(v[0])\n\
                               \x20   println(w[0])\n\
                               \x20   return 0\n\
                               }\n";

#[test]
fn the_direct_spelling_still_detaches() {
    let h = Harness::new();
    h.value_row("C07-direct", DIRECT_SPELLING, "101\n7919\n", 2, 2);
    h.assert_legs(8);
}


#[test]
fn e0416_names_the_borrow_and_the_requirement() {
    let h = Harness::new();
    let rendered = h.fenced_row("R1-render", R1_LET, "E0416");
    for needle in ["shared borrow", "`&i64`", "`&mut i64`", "required"] {
        assert!(
            rendered.contains(needle),
            "E0416 must name {needle} in the user's own types, not a bare unification failure:\n\
             {rendered}"
        );
    }
    assert!(
        !rendered.contains("E0301"),
        "the reference model refusal must not fall through to the generic mismatch code:\n\
         {rendered}"
    );
    h.assert_legs(4);
}

#[test]
fn e0416_is_registered_and_explainable() {
    let info = aelys_common::diagnostic::registry::lookup("E0416")
        .expect("E0416 must be registered; E0410 and E0411 are live unregistered codes already");
    assert!(
        info.explanation.contains("&mut"),
        "the --explain body must show the distinction it is enforcing"
    );
    assert!(
        info.explanation.contains("let r: &i64 = &mut x"),
        "the --explain body must show the weakening that stays accepted, or it reads as a ban \
         on references"
    );
}


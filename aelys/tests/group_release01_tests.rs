use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
];

const ALLOCATORS: &[(&str, Option<&str>)] = &[("immix", None), ("malloc", Some("malloc"))];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn copy_tree(from: &Path, to: &Path) {
    fs::create_dir_all(to).expect("create staged directory");
    for entry in fs::read_dir(from).expect("read library directory") {
        let entry = entry.expect("read library entry");
        let target = to.join(entry.file_name());
        if entry.file_type().expect("library entry kind").is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), &target).expect("copy library file");
        }
    }
}

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
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

fn parse_stats(stderr: &str) -> Option<(i64, i64)> {
    let line = stderr.lines().find(|l| l.contains("[rc] allocs="))?;
    let rest = line.trim().strip_prefix("[rc] allocs=")?;
    let (a, m) = rest.split_once(" frees=")?;
    Some((a.trim().parse().ok()?, m.trim().parse().ok()?))
}

fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found") || error.contains("failed to run")
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

struct Harness {
    dir: TempDir,
    legs: Cell<usize>,
    linker_skips: Cell<usize>,
}

impl Harness {
    fn new() -> Self {
        Harness {
            dir: tempdir().expect("tempdir"),
            legs: Cell::new(0),
            linker_skips: Cell::new(0),
        }
    }

    fn stage(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let dir = self.dir.path().join(slug(id, tag));
        fs::create_dir_all(&dir).expect("stage dir");
        copy_tree(&repo_root().join("std"), &dir.join("std"));
        let root = dir.join("root.aelys");
        fs::write(&root, src).expect("write root fixture");
        root
    }

    fn cycles_row(&self, id: &str, src: &str, stdout: &str, counted: (i64, i64)) {
        for (tag, opt) in LEVELS {
            for (alloc_name, alloc) in ALLOCATORS {
                let leg = format!("{tag}_{alloc_name}");
                let root = self.stage(id, &leg, src);
                if let Err(err) =
                    compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::RcCycles)
                {
                    if linker_unavailable(&err.to_string()) {
                        self.linker_skips.set(self.linker_skips.get() + 1);
                        assert!(
                            linker_skip_declared(),
                            "{id}: no linker and AELYS_ALLOW_LINKER_SKIP is unset"
                        );
                        return;
                    }
                    panic!("{id} at {leg}: compile\n{err}");
                }
                let mut cmd = Command::new(exe_path_for(&root));
                cmd.env("AELYS_RC_STATS", "1");
                if let Some(a) = alloc {
                    cmd.env("AELYS_ALLOC", a);
                }
                let out = cmd.output().expect("run compiled exe");
                let seen_err = String::from_utf8_lossy(&out.stderr).into_owned();
                self.legs.set(self.legs.get() + 1);
                assert_eq!(exit_code(&out.status), 0, "{id} at {leg}: exit\n{seen_err}");
                assert_eq!(
                    String::from_utf8_lossy(&out.stdout),
                    stdout,
                    "{id} at {leg}: answer"
                );
                let got = parse_stats(&seen_err)
                    .unwrap_or_else(|| panic!("{id} at {leg}: no [rc] stats line\n{seen_err}"));
                assert_eq!(got, counted, "{id} at {leg}: allocs and frees");
            }
        }
    }

    fn assert_legs(&self, expected: usize) {
        if self.linker_skips.get() > 0 && linker_skip_declared() {
            return;
        }
        assert_eq!(self.legs.get(), expected, "legs run");
    }
}

const RELEASE01_1: &str = r#"
struct Leaf { k: i64 }
enum E { A(i32, Rc<Leaf>) }
struct N { v: i64, e: E }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    match Rc::get(a).e {
        E::A(x, r) => { return x + Rc::get(r).k }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, e: E::A(20, Rc::new(Leaf { k: 300 })) })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_1_an_rc_payload_field_after_an_i32_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-1", RELEASE01_1, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_2: &str = r#"
struct Leaf { k: i64 }
enum E { A(u8, Rc<Leaf>) }
struct N { v: i64, e: E }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    match Rc::get(a).e {
        E::A(x, r) => { return x + Rc::get(r).k }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, e: E::A(20, Rc::new(Leaf { k: 300 })) })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_2_an_rc_payload_field_after_a_u8_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-2", RELEASE01_2, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_3: &str = r#"
struct Leaf { k: i64 }
enum E { A(i64, Rc<Leaf>) }
struct N { v: i64, e: E }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    match Rc::get(a).e {
        E::A(x, r) => { return x + Rc::get(r).k }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, e: E::A(20, Rc::new(Leaf { k: 300 })) })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_3_an_rc_payload_field_after_an_i64_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-3", RELEASE01_3, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_4: &str = r#"
struct Leaf { k: i64 }
enum E { A(Rc<Leaf>, i32) }
struct N { v: i64, e: E }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    match Rc::get(a).e {
        E::A(r, x) => { return Rc::get(r).k + x }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, e: E::A(Rc::new(Leaf { k: 300 }), 20) })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_4_an_rc_first_payload_field_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-4", RELEASE01_4, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_5: &str = r#"
struct Leaf { k: i64 }
enum E { A(Rc<Leaf>) }
struct Inner { tag: u8, e: E }
struct N { v: i64, w: i32, inner: Inner }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    let n = Rc::get(a)
    match n.inner.e {
        E::A(r) => { return n.w + Rc::get(r).k }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, w: 20, inner: Inner { tag: 7, e: E::A(Rc::new(Leaf { k: 300 })) } })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_5_an_enum_two_struct_levels_deep_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-5", RELEASE01_5, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_6: &str = r#"
struct Leaf { k: i64 }
struct P { k: i32, r: Rc<Leaf> }
enum E { A(P) }
struct N { v: i64, e: E }
fn peek(a: Rc<N>) -> i64 {
    let c = a
    return Rc::get(c).v
}
fn read(a: Rc<N>) -> i64 {
    match Rc::get(a).e {
        E::A(p) => { return p.k + Rc::get(p.r).k }
    }
}
fn work() -> i64 {
    let a: Rc<N> = Rc::new(N { v: 1, e: E::A(P { k: 20, r: Rc::new(Leaf { k: 300 }) }) })
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_6_a_struct_in_an_enum_payload_carrying_an_rc_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-6", RELEASE01_6, "321\n", (2, 2));
    h.assert_legs(4);
}

const RELEASE01_7: &str = r#"
struct Leaf { k: i64 }
enum E { A(i32, Rc<Leaf>) }
fn peek(a: Rc<E>) -> i64 {
    let c = a
    return 1
}
fn read(a: Rc<E>) -> i64 {
    match Rc::get(a) {
        E::A(x, r) => { return x + Rc::get(r).k }
    }
}
fn work() -> i64 {
    let a: Rc<E> = Rc::new(E::A(20, Rc::new(Leaf { k: 300 })))
    let t = peek(a)
    __aelys_collect()
    return t + read(a)
}
fn main() -> i64 {
    let t = work()
    __aelys_collect()
    println("{t}")
    return 0
}
"#;

#[test]
fn release01_7_an_enum_held_directly_in_an_rc_node_is_traced_at_its_real_offset() {
    let h = Harness::new();
    h.cycles_row("release01-7", RELEASE01_7, "321\n", (2, 2));
    h.assert_legs(4);
}

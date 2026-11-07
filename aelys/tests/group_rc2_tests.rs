mod common;

use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Once, OnceLock};
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const ALLOCATORS: &[(&str, Option<&str>)] = &[("immix", None), ("malloc", Some("malloc"))];

const MODULES: &[&str] = &["result.aelys", "str.aelys"];

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

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

fn library_root() -> PathBuf {
    let dir = repo_root().join("std");
    for module in MODULES {
        let file = dir.join(module);
        let size = fs::metadata(&file)
            .unwrap_or_else(|e| panic!("std/{module} at {}: {e}", file.display()))
            .len();
        assert!(size > 0, "std/{module} is empty");
    }
    dir
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
    library: PathBuf,
    legs: Cell<usize>,
    linker_skips: Cell<usize>,
}

impl Harness {
    fn new() -> Self {
        warm_core_archive();
        Harness {
            dir: tempdir().expect("tempdir"),
            library: library_root(),
            legs: Cell::new(0),
            linker_skips: Cell::new(0),
        }
    }

    fn stage(&self, id: &str, tag: &str, src: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = self.dir.path().join(slug(id, tag));
        fs::create_dir_all(&dir).expect("stage dir");
        copy_tree(&self.library, &dir.join("std"));
        for (name, body) in files {
            fs::write(dir.join(name), body).expect("write module fixture");
        }
        let root = dir.join("root.aelys");
        fs::write(&root, src).expect("write root fixture");
        root
    }

    fn compile_at(
        &self,
        id: &str,
        tag: &str,
        root: &Path,
        opt: OptimizationLevel,
        variant: RuntimeVariant,
    ) -> Option<PathBuf> {
        match compile_file_with_llvm_variant(root, opt, false, variant) {
            Ok(()) => {}
            Err(err) => {
                if linker_unavailable(&err.to_string()) {
                    self.linker_skips.set(self.linker_skips.get() + 1);
                    return None;
                }
                panic!("{id} at {tag}: compile\n{err}");
            }
        }
        let exe = exe_path_for(root);
        exe.is_file().then_some(exe)
    }

    fn differential_row(
        &self,
        id: &str,
        src: &str,
        files: &[(&str, &str)],
        stdout: &str,
        counted: (i64, i64),
    ) {
        const VARIANTS: &[(&str, RuntimeVariant)] = &[
            ("leak", RuntimeVariant::Leak),
            ("rc", RuntimeVariant::Rc),
            ("rc+cycles", RuntimeVariant::RcCycles),
        ];
        for (tag, opt) in LEVELS {
            for (alloc_name, alloc) in ALLOCATORS {
                let mut answers: Vec<String> = Vec::new();
                let mut seen_counts: Vec<(&str, (i64, i64))> = Vec::new();
                for (runtime, variant) in VARIANTS {
                    let leg = format!("{tag}_{alloc_name}_{runtime}");
                    let root = self.stage(id, &leg, src, files);
                    let Some(exe) = self.compile_at(id, &leg, &root, *opt, *variant) else {
                        assert!(
                            linker_skip_declared(),
                            "{id}: no linker and AELYS_ALLOW_LINKER_SKIP is unset"
                        );
                        return;
                    };
                    let mut cmd = Command::new(&exe);
                    cmd.env("AELYS_RC_STATS", "1");
                    if let Some(a) = alloc {
                        cmd.env("AELYS_ALLOC", a);
                    }
                    let out = cmd.output().expect("run compiled exe");
                    let seen_out = String::from_utf8_lossy(&out.stdout).into_owned();
                    let seen_err = String::from_utf8_lossy(&out.stderr).into_owned();
                    self.legs.set(self.legs.get() + 1);
                    assert_eq!(
                        exit_code(&out.status),
                        0,
                        "{id} at {tag}/{alloc_name}/{runtime}: exit\n{seen_err}"
                    );
                    if *runtime != "leak" {
                        let got = parse_stats(&seen_err).unwrap_or_else(|| {
                            panic!(
                                "{id} at {tag}/{alloc_name}/{runtime}: no [rc] stats line\n\
                                 stderr:\n{seen_err}"
                            )
                        });
                        seen_counts.push((runtime, got));
                    }
                    answers.push(seen_out);
                }
                for (i, (runtime, _)) in VARIANTS.iter().enumerate().skip(1) {
                    assert_eq!(
                        answers[i], answers[0],
                        "{id} at {tag}/{alloc_name}: {runtime} answers {:?}, leak answers {:?}",
                        answers[i], answers[0]
                    );
                }
                assert_eq!(
                    answers[0], stdout,
                    "{id} at {tag}/{alloc_name}: leak answer"
                );
                for (runtime, got) in &seen_counts {
                    assert_eq!(
                        *got, counted,
                        "{id} at {tag}/{alloc_name}/{runtime}: allocs and frees"
                    );
                }
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

static ASAN_DIR: OnceLock<PathBuf> = OnceLock::new();

fn asan_archives() -> PathBuf {
    ASAN_DIR
        .get_or_init(|| {
            let dir =
                std::env::temp_dir().join(format!("aelys-group-rc2-asan-{}", std::process::id()));
            fs::create_dir_all(&dir).expect("asan dir");
            let core = repo_root().join("core").join("src");
            for (lib, rc) in [
                ("core-rc", "aelys_rc_real.c"),
                ("core-leak", "aelys_rc_leak.c"),
            ] {
                let mut objects = Vec::new();
                for unit in ["aelys_core_common.c", "aelys_alloc_immix.c", rc] {
                    let obj = dir.join(format!("{lib}-{unit}")).with_extension("o");
                    let out = Command::new("clang")
                        .args(["-fsanitize=address", "-g", "-c"])
                        .arg(core.join(unit))
                        .arg(format!("-I{}", core.display()))
                        .arg("-o")
                        .arg(&obj)
                        .output()
                        .expect("clang");
                    assert!(
                        out.status.success(),
                        "{unit}: {}",
                        String::from_utf8_lossy(&out.stderr)
                    );
                    objects.push(obj);
                }
                let out = Command::new("ar")
                    .arg("rcs")
                    .arg(dir.join(format!("lib{lib}.a")))
                    .args(&objects)
                    .output()
                    .expect("ar");
                assert!(
                    out.status.success(),
                    "ar: {}",
                    String::from_utf8_lossy(&out.stderr)
                );
            }
            dir
        })
        .clone()
}

impl Harness {
    fn lsan_run(
        &self,
        id: &str,
        leg: &str,
        src: &str,
        files: &[(&str, &str)],
        opt: OptimizationLevel,
        variant: RuntimeVariant,
    ) -> (i32, String, String) {
        let root = self.stage(id, leg, src, files);
        common::with_outline_str_counts(|| {
            compile_file_with_llvm_variant(&root, opt, false, variant)
        })
        .unwrap_or_else(|e| panic!("{id} at {leg}: compile\n{e}"));
        let object = root.with_extension("o");
        assert!(object.is_file(), "{id} at {leg}: the driver left no object");
        let lib = if matches!(variant, RuntimeVariant::Leak) {
            "core-leak"
        } else {
            "core-rc"
        };
        let exe = root.with_file_name("asan_exe");
        let link = Command::new("clang")
            .args(["-fsanitize=address", "-g"])
            .arg(&object)
            .arg(format!("-L{}", asan_archives().display()))
            .arg(format!("-l{lib}"))
            .arg("-o")
            .arg(&exe)
            .output()
            .expect("clang link");
        assert!(
            link.status.success(),
            "{id}: link failed:\n{}",
            String::from_utf8_lossy(&link.stderr)
        );
        let out = Command::new(&exe)
            .env("AELYS_ALLOC", "malloc")
            .env(
                "ASAN_OPTIONS",
                "detect_leaks=1:verbosity=1:max_free_fill_size=4096:free_fill_byte=170",
            )
            .output()
            .expect("run asan exe");
        self.legs.set(self.legs.get() + 1);
        (
            exit_code(&out.status),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn lsan_row(&self, id: &str, src: &str, files: &[(&str, &str)], stdout: &str) {
        for (tag, opt) in LEVELS {
            let (code, out, err) = self.lsan_run(
                id,
                &format!("{tag}_lsan_rc"),
                src,
                files,
                *opt,
                RuntimeVariant::Rc,
            );
            assert!(
                err.contains("AddressSanitizer"),
                "{id} at {tag}: asan is not armed"
            );
            assert!(
                !err.contains("ERROR: AddressSanitizer"),
                "{id} at {tag}: asan reports\n{err}"
            );
            assert!(
                !err.contains("leaked in"),
                "{id} at {tag}: lsan reports a leak\n{err}"
            );
            assert_eq!(code, 0, "{id} at {tag}: exit\n{err}");
            assert_eq!(out, stdout, "{id} at {tag}");
            let (_, _, err) = self.lsan_run(
                id,
                &format!("{tag}_lsan_leak"),
                src,
                files,
                *opt,
                RuntimeVariant::Leak,
            );
            assert!(
                err.contains("leaked in"),
                "{id} at {tag}: lsan silent under the leak runtime\n{err}"
            );
        }
    }
}

fn named_callees(function: &aelys_air::AirFunction) -> Vec<String> {
    let mut out = Vec::new();
    for block in &function.blocks {
        for stmt in &block.stmts {
            match &stmt.kind {
                aelys_air::AirStmtKind::CallVoid {
                    func: aelys_air::Callee::Named(name),
                    ..
                }
                | aelys_air::AirStmtKind::Assign {
                    rvalue:
                        aelys_air::Rvalue::Call {
                            func: aelys_air::Callee::Named(name),
                            ..
                        },
                    ..
                } => out.push(name.clone()),
                _ => {}
            }
        }
    }
    out
}

fn str_retains_in(src: &str, function: &str) -> usize {
    calls_in(src, function, "__aelys_str_retain")
}

fn calls_in(src: &str, function: &str, callee: &str) -> usize {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, src).expect("write root fixture");
    let air = aelys_driver::lower_file_to_air(&root, OptimizationLevel::None)
        .unwrap_or_else(|e| panic!("lower: {e}"));
    let f = air
        .functions
        .iter()
        .find(|f| f.name == function)
        .unwrap_or_else(|| panic!("no function `{function}`"));
    named_callees(f)
        .iter()
        .filter(|c| c.as_str() == callee)
        .count()
}

const RC2_1: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct V { s: string }
struct W { next: Rc<V> }
fn clobber(h: Rc<W>, s: string) -> i64 {
    h.next = Rc::new(V { s: victim(1) })
    let t: Rc<V> = Rc::new(V { s: other(88888) })
    return s.bytes[0] + s.len + Rc::get(t).s.len
}
fn main() -> i64 {
    let a = Rc::new(W { next: Rc::new(V { s: victim(12345) }) })
    println("{clobber(a, Rc::get(Rc::get(a).next).s)}")
    return 0
}
"#;

#[test]
fn rc2_1_a_string_read_through_an_inner_rc_stays_alive_while_the_callee_replaces_that_rc() {
    let h = Harness::new();
    h.differential_row("rc2-1", RC2_1, &[], "111\n", (10, 10));
    h.lsan_row("rc2-1", RC2_1, &[], "111\n");
    h.assert_legs(24);
}

const RC2_2: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn clobber(h: Rc<W>, s: string) -> i64 {
    let r = &mut Rc::get(h)
    (*r).s = victim(1)
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    println("{clobber(a, Rc::get(a).s)}")
    return 0
}
"#;

#[test]
fn rc2_2_a_payload_string_argument_survives_a_callee_writing_through_a_mutable_reference() {
    let h = Harness::new();
    h.differential_row("rc2-2", RC2_2, &[], "111\n", (7, 7));
    h.lsan_row("rc2-2", RC2_2, &[], "111\n");
    h.assert_legs(24);
}

const RC2_3: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    let b = a
    let bs = Rc::get(a).s.bytes
    let r = &mut Rc::get(b)
    (*r).s = victim(1)
    let t: string = other(88888)
    println("{bs[0] + bs.len + t.len}")
    return 0
}
"#;

#[test]
fn rc2_3_a_bytes_view_of_a_payload_string_survives_a_write_through_a_mutable_reference() {
    let h = Harness::new();
    h.differential_row("rc2-3", RC2_3, &[], "111\n", (7, 7));
    h.lsan_row("rc2-3", RC2_3, &[], "111\n");
    h.assert_legs(24);
}

const RC2_4: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct V { s: string, a: [string; 2] }
struct W { next: Rc<V> }
fn clobber(h: Rc<W>) -> i64 {
    h.next = Rc::new(V { s: victim(1), a: [victim(1), victim(2)] })
    return 0
}
fn main() -> i64 {
    let a = Rc::new(W { next: Rc::new(V { s: victim(12345), a: [victim(1), victim(2)] }) })
    let bs = a.next.s.bytes
    let z = clobber(a)
    let t: Rc<V> = Rc::new(V { s: other(88888), a: [victim(1), victim(2)] })
    println("{bs[0] + bs.len + z + t.s.len}")
    return 0
}
"#;

#[test]
fn rc2_4_a_bytes_view_through_an_implicit_inner_rc_survives_that_rc_being_replaced() {
    let h = Harness::new();
    h.differential_row("rc2-4", RC2_4, &[], "111\n", (22, 22));
    h.lsan_row("rc2-4", RC2_4, &[], "111\n");
    h.assert_legs(24);
}

const RC2_5: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct V { s: string, a: [string; 2] }
struct W { next: Rc<V> }
fn clobber(h: Rc<W>) -> i64 {
    h.next = Rc::new(V { s: victim(1), a: [victim(1), victim(2)] })
    return 0
}
fn clob2(h: Rc<W>, n: Rc<V>) -> i64 {
    let z = clobber(h)
    let t: Rc<V> = Rc::new(V { s: other(88888), a: [victim(1), victim(2)] })
    return n.s.bytes[0] + n.s.len + z + t.s.len
}
fn main() -> i64 {
    let a = Rc::new(W { next: Rc::new(V { s: victim(12345), a: [victim(1), victim(2)] }) })
    println("{clob2(a, a.next)}")
    return 0
}
"#;

#[test]
fn rc2_5_an_inner_rc_argument_read_by_implicit_deref_stays_alive_for_the_call() {
    let h = Harness::new();
    h.differential_row("rc2-5", RC2_5, &[], "111\n", (22, 22));
    h.lsan_row("rc2-5", RC2_5, &[], "111\n");
    h.assert_legs(24);
}

const RC2_6: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct V { s: string }
struct W { next: Rc<V> }
fn clobber(h: Rc<W>, n: Rc<V>) -> i64 {
    h.next = Rc::new(V { s: victim(1) })
    let t: Rc<V> = Rc::new(V { s: other(88888) })
    return Rc::get(n).s.bytes[0] + Rc::get(n).s.len + Rc::get(t).s.len
}
fn main() -> i64 {
    let a = Rc::new(W { next: Rc::new(V { s: victim(12345) }) })
    println("{clobber(a, Rc::get(a).next)}")
    return 0
}
"#;

#[test]
fn rc2_6_an_inner_rc_argument_read_through_rc_get_stays_alive_for_the_call() {
    let h = Harness::new();
    h.differential_row("rc2-6", RC2_6, &[], "111\n", (10, 10));
    h.lsan_row("rc2-6", RC2_6, &[], "111\n");
    h.assert_legs(24);
}

const RC2_7: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn clobber(h: Rc<W>, s: string) -> i64 {
    let r = &mut Rc::get(h)
    *r = W { s: victim(1) }
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    println("{clobber(a, Rc::get(a).s)}")
    return 0
}
"#;

#[test]
fn rc2_7_a_payload_string_argument_survives_the_whole_payload_being_overwritten() {
    let h = Harness::new();
    h.differential_row("rc2-7", RC2_7, &[], "111\n", (7, 7));
    h.lsan_row("rc2-7", RC2_7, &[], "111\n");
    h.assert_legs(24);
}

const RC2_8: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
let mut G: string = "seed"
nogc fn w() { G = "q" }
nogc fn k(s: string) -> i64 {
    w()
    return s.bytes[0] + s.len
}
nogc fn f() -> i64 { return k(G) }
fn main() -> i64 {
    G = victim(12345)
    let n = f()
    let t: string = other(88888)
    println("{n + t.len}")
    return 0
}
"#;

#[test]
fn rc2_8_a_nogc_body_keeps_its_share_of_a_global_argument() {
    assert!(str_retains_in(RC2_8, "f") > 0, "no str_retain in nogc `f`");
    let h = Harness::new();
    h.differential_row("rc2-8", RC2_8, &[], "111\n", (4, 4));
    h.lsan_row("rc2-8", RC2_8, &[], "111\n");
    h.assert_legs(24);
}

const RC2_9: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct P { s: string, t: string }
nogc fn k(p: &mut P, x: string) -> i64 {
    (*p).s = (*p).t
    return x.bytes[0] + x.len
}
nogc fn f(p: &mut P) -> i64 { return k(p, (*p).s) }
fn main() -> i64 {
    let mut p = P { s: victim(12345), t: victim(1) }
    let n = f(&mut p)
    let t: string = other(88888)
    println("{n + t.len}")
    return 0
}
"#;

#[test]
fn rc2_9_a_nogc_body_keeps_its_snapshot_of_an_argument_beside_a_mutable_reference() {
    assert!(str_retains_in(RC2_9, "f") > 0, "no str_retain in nogc `f`");
    let h = Harness::new();
    h.differential_row("rc2-9", RC2_9, &[], "111\n", (6, 6));
    h.lsan_row("rc2-9", RC2_9, &[], "111\n");
    h.assert_legs(24);
}

const RC2_10: &str = r#"
struct P { s: string }
nogc fn k(x: string) -> i64 { return x.len }
nogc fn f(p: &P) -> i64 { return k((*p).s) }
fn g(p: &P) -> i64 { return k((*p).s) }
fn main() -> i64 {
    let p = P { s: "ab" }
    println("{f(&p) + g(&p)}")
    return 0
}
"#;

#[test]
fn rc2_10_a_nogc_body_takes_no_new_share_for_an_argument_read_through_a_reference() {
    assert_eq!(str_retains_in(RC2_10, "f"), 0, "str_retain in nogc `f`");
    assert!(str_retains_in(RC2_10, "g") > 0, "no str_retain in `g`");
}

const RC2_11: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn clob(h: Rc<W>, n: i64) -> i64 {
    let r = &mut Rc::get(h)
    (*r).s = victim(n)
    return 0
}
fn by(w: &W, h: Rc<W>) -> &[u8] {
    let z = clob(h, 1)
    return (*w).s.bytes
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    let bs = by(&Rc::get(a), a)
    let z = clob(a, 777777)
    let t: string = other(88888)
    println("{bs[0] + bs.len + t.len + z}")
    return 0
}
"#;

#[test]
fn rc2_11_a_bound_byte_view_holds_the_string_the_callee_installed_before_returning_it() {
    let h = Harness::new();
    h.differential_row("rc2-11", RC2_11, &[], "107\n", (9, 9));
    h.lsan_row("rc2-11", RC2_11, &[], "107\n");
    h.assert_legs(24);
}

const RC2_12: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn clob(h: Rc<W>, n: i64) -> i64 {
    let r = &mut Rc::get(h)
    (*r).s = victim(n)
    return 0
}
fn by(w: &W, h: Rc<W>) -> &[u8] {
    let z = clob(h, 1)
    return (*w).s.bytes
}
fn use(h: Rc<W>, v: &[u8]) -> i64 {
    let z = clob(h, 777777)
    let t: string = other(88888)
    return v[0] + v.len + t.len + z
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    println("{use(a, by(&Rc::get(a), a))}")
    return 0
}
"#;

#[test]
fn rc2_12_a_byte_view_argument_holds_the_string_the_callee_installed_before_returning_it() {
    let h = Harness::new();
    h.differential_row("rc2-12", RC2_12, &[], "107\n", (9, 9));
    h.lsan_row("rc2-12", RC2_12, &[], "107\n");
    h.assert_legs(24);
}

const RC2_13: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn clob(h: Rc<W>, n: i64) -> i64 {
    let r = &mut Rc::get(h)
    (*r).s = victim(n)
    return 0
}
fn by(w: &W, h: Rc<W>) -> &[u8] {
    let z = clob(h, 1)
    return (*w).s.bytes
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    let mut n: i64 = 0
    for b in by(&Rc::get(a), a) {
        let z = clob(a, 777777)
        let t: string = other(88888)
        n = n + b + z
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn rc2_13_a_for_over_a_byte_view_holds_the_string_the_callee_installed_before_returning_it() {
    let h = Harness::new();
    h.differential_row("rc2-13", RC2_13, &[], "244\n", (17, 17));
    h.lsan_row("rc2-13", RC2_13, &[], "244\n");
    h.assert_legs(24);
}

const RC2_14: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
let mut G: string = "seed"
nogc fn w() { G = "q" }
nogc fn f() -> i64 {
    let bs = G.bytes
    w()
    return bs[0] + bs.len
}
fn main() -> i64 {
    G = victim(12345)
    let n = f()
    let t: string = other(88888)
    println("{n + t.len}")
    return 0
}
"#;

#[test]
fn rc2_14_a_nogc_bytes_view_of_a_global_survives_the_global_being_replaced() {
    let h = Harness::new();
    h.differential_row("rc2-14", RC2_14, &[], "111\n", (4, 4));
    h.lsan_row("rc2-14", RC2_14, &[], "111\n");
    h.assert_legs(24);
}

const RC2_15: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
let mut G: string = "seed"
nogc fn w() { G = "q" }
nogc fn f() -> i64 {
    let mut n: i64 = 0
    for b in G.bytes {
        w()
        n = n + b
    }
    return n
}
fn main() -> i64 {
    G = victim(12345)
    let n = f()
    let t: string = other(88888)
    println("{n + t.len}")
    return 0
}
"#;

#[test]
fn rc2_15_a_nogc_for_over_the_bytes_of_a_global_survives_the_global_being_replaced() {
    let h = Harness::new();
    h.differential_row("rc2-15", RC2_15, &[], "457\n", (4, 4));
    h.lsan_row("rc2-15", RC2_15, &[], "457\n");
    h.assert_legs(24);
}

const RC2_16: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
enum Tag { Named(string), Anon }
struct Item { name: string, tag: Tag }
fn name_bytes(it: &Item) -> &[u8] { return (*it).name.bytes }
fn count(it: &Item) -> i64 {
    let bs = name_bytes(it)
    return bs.len
}
fn main() -> i64 {
    let it = Item { name: victim(12345), tag: Tag::Named(victim(1)) }
    println("{count(&it)}")
    return 0
}
"#;

#[test]
fn rc2_16_a_byte_view_from_a_reference_to_a_struct_holding_an_enum_is_accepted() {
    let h = Harness::new();
    h.differential_row("rc2-16", RC2_16, &[], "7\n", (4, 4));
    h.lsan_row("rc2-16", RC2_16, &[], "7\n");
    h.assert_legs(24);
}

const RC2_17: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
struct Rec { a: string, b: string, c: string, d: string, e: string, f: string, g: string, h: string, i: string }
fn first(r: &Rec) -> &[u8] { return (*r).a.bytes }
fn count(r: &Rec) -> i64 {
    let mut n: i64 = 0
    for b in first(r) { n = n + 1 }
    return n
}
fn main() -> i64 {
    let r = Rec { a: victim(1), b: victim(2), c: victim(3), d: victim(4), e: victim(5), f: victim(6), g: victim(7), h: victim(8), i: victim(9) }
    println("{count(&r)}")
    return 0
}
"#;

#[test]
fn rc2_17_a_byte_view_from_a_reference_to_a_struct_of_nine_strings_is_accepted() {
    let h = Harness::new();
    h.differential_row("rc2-17", RC2_17, &[], "3\n", (18, 18));
    h.lsan_row("rc2-17", RC2_17, &[], "3\n");
    h.assert_legs(24);
}

const RC2_18: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
enum Tag { Named(string), Anon }
fn idr<T>(x: &T) -> &T { return x }
fn count(t: &Tag) -> i64 {
    let r = idr(t)
    return match *r { Tag::Named(s) => s.len, Tag::Anon => 0 }
}
fn main() -> i64 {
    let t = Tag::Named(victim(12345))
    println("{count(&t)}")
    return 0
}
"#;

#[test]
fn rc2_18_a_reference_returned_through_a_generic_identity_is_accepted() {
    let h = Harness::new();
    h.differential_row("rc2-18", RC2_18, &[], "7\n", (2, 2));
    h.lsan_row("rc2-18", RC2_18, &[], "7\n");
    h.assert_legs(24);
}

const RC2_19: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
enum E { A(string), B }
struct W { s: string, e: E }
fn first(w: &W) -> &[u8] { return (*w).s.bytes }
fn main() -> i64 {
    let w = W { s: victim(12345), e: E::A(victim(2)) }
    let p = &w
    let bs = first(p)
    let t: string = other(88888)
    println("{bs[0] + bs.len + t.len}")
    return 0
}
"#;

#[test]
fn rc2_19_a_byte_view_through_a_local_reference_to_an_enum_carrier_is_accepted() {
    let h = Harness::new();
    h.differential_row("rc2-19", RC2_19, &[], "111\n", (6, 6));
    h.lsan_row("rc2-19", RC2_19, &[], "111\n");
    h.assert_legs(24);
}

const RC2_20: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn head(xs: &[string]) -> &[u8] { return xs[0].bytes }
fn count(xs: &[string]) -> i64 {
    let bs = head(xs)
    return bs.len
}
fn main() -> i64 {
    let arr: [string; 2] = [victim(12345), victim(2)]
    println("{count(arr[0..2])}")
    return 0
}
"#;

#[test]
fn rc2_20_a_byte_view_from_a_slice_of_strings_the_function_does_not_own_is_refused() {
    let h = Harness::new();
    let root = h.stage("rc2-20", "refuse", RC2_20, &[]);
    let err =
        compile_file_with_llvm_variant(&root, OptimizationLevel::None, false, RuntimeVariant::Rc)
            .expect_err("rc2-20 compiled");
    assert!(
        err.to_string().contains(
            "a byte view returned from a slice of strings this function does not own is not \
             supported yet"
        ),
        "rc2-20: diagnostic\n{err}"
    );
}

const RC2_21: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn clob(h: Rc<W>) -> string {
    let r = &mut Rc::get(h)
    (*r).s = victim(1)
    let t: string = other(88888)
    return ""
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    a.s += clob(a)
    let t: string = other(77777)
    println("{a.s.bytes[0] + a.s.len + t.len}")
    return 0
}
"#;

#[test]
fn rc2_21_a_compound_field_append_keeps_the_old_value_while_the_right_side_replaces_it() {
    let h = Harness::new();
    h.differential_row("rc2-21", RC2_21, &[], "111\n", (10, 10));
    h.lsan_row("rc2-21", RC2_21, &[], "111\n");
    h.assert_legs(24);
}

const RC2_22: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { a: [string; 2] }
fn clob(h: Rc<W>) -> string {
    let r = &mut Rc::get(h)
    (*r).a[1] = victim(1)
    let t: string = other(88888)
    return ""
}
fn main() -> i64 {
    let a = Rc::new(W { a: [victim(1), victim(12345)] })
    a.a[1] += clob(a)
    let t: string = other(77777)
    println("{a.a[1].bytes[0] + a.a[1].len + t.len}")
    return 0
}
"#;

#[test]
fn rc2_22_a_compound_element_append_keeps_the_old_value_while_the_right_side_replaces_it() {
    let h = Harness::new();
    h.differential_row("rc2-22", RC2_22, &[], "111\n", (12, 12));
    h.lsan_row("rc2-22", RC2_22, &[], "111\n");
    h.assert_legs(24);
}

const RC2_23: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
let mut G: string = "seed"
fn setg() -> i64 {
    G = victim(1)
    return 0
}
fn main() -> i64 {
    G = victim(12345)
    let bs = G.bytes
    let z = setg()
    let t: string = other(88888)
    println("{bs[0] + bs.len + t.len + z}")
    return 0
}
"#;

#[test]
fn rc2_23_a_bytes_view_of_a_global_survives_a_call_that_replaces_the_global() {
    let h = Harness::new();
    h.differential_row("rc2-23", RC2_23, &[], "111\n", (6, 5));
    h.lsan_row("rc2-23", RC2_23, &[], "111\n");
    h.assert_legs(24);
}

const RC2_24: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn hb(v: &Vec<string>) -> &[u8] { return (*v)[0].bytes }
fn count(v: &Vec<string>) -> i64 {
    let bs = hb(v)
    return bs.len + bs[0]
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(12345))
    let n = count(&v)
    Vec::push(v, victim(7))
    let w = v
    Vec::push(w, victim(8))
    println("{n} {hb(&v).len} {v.len} {w.len} {v[1]} {w[2]}")
    return 0
}
"#;

#[test]
fn rc2_24_a_byte_view_from_a_vector_of_strings_holds_the_buffer_and_leaves_copy_on_write_alone() {
    assert!(
        calls_in(RC2_24, "count", "__aelys_vec_retain") > 0,
        "no vec share in `count`"
    );
    let h = Harness::new();
    h.differential_row("rc2-24", RC2_24, &[], "104 7 2 3 ab7 ab8\n", (22, 22));
    h.lsan_row("rc2-24", RC2_24, &[], "104 7 2 3 ab7 ab8\n");
    h.assert_legs(24);
}

const RC2_25: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct In { s: string }
struct W { i: In }
fn clobber(h: Rc<W>, s: string) -> i64 {
    h.i.s = victim(1)
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn main() -> i64 {
    let a = Rc::new(W { i: In { s: victim(12345) } })
    println("{clobber(a, Rc::get(a).i.s)}")
    return 0
}
"#;

#[test]
fn rc2_25_a_nested_field_argument_read_through_rc_get_survives_the_callee_replacing_it() {
    let h = Harness::new();
    h.differential_row("rc2-25", RC2_25, &[], "111\n", (7, 7));
    h.lsan_row("rc2-25", RC2_25, &[], "111\n");
    h.assert_legs(24);
}

const RC2_26: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn clobber(h: Rc<W>, s: string) -> i64 {
    h.s = victim(1)
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    println("{clobber(a, a.s)}")
    return 0
}
"#;

#[test]
fn rc2_26_a_field_argument_read_through_the_handle_survives_the_callee_replacing_it() {
    let h = Harness::new();
    h.differential_row("rc2-26", RC2_26, &[], "111\n", (7, 7));
    h.lsan_row("rc2-26", RC2_26, &[], "111\n");
    h.assert_legs(24);
}

const RC2_27: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    let b = a
    let bs = a.s.bytes
    b.s = victim(1)
    let t: string = other(88888)
    println("{bs[0] + bs.len + t.len}")
    return 0
}
"#;

#[test]
fn rc2_27_a_bytes_view_of_a_field_read_through_the_handle_survives_a_store_through_an_alias() {
    let h = Harness::new();
    h.differential_row("rc2-27", RC2_27, &[], "111\n", (7, 7));
    h.lsan_row("rc2-27", RC2_27, &[], "111\n");
    h.assert_legs(24);
}

const RC2_28: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct In { s: string }
struct W { i: In }
fn clobber(h: Rc<W>, s: string) -> i64 {
    h.i = In { s: victim(1) }
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn mid(h: Rc<W>) -> i64 { return clobber(h, h.i.s) }
fn main() -> i64 {
    let a = Rc::new(W { i: In { s: victim(12345) } })
    println("{mid(a)}")
    return 0
}
"#;

#[test]
fn rc2_28_a_nested_field_argument_through_a_parameter_handle_survives_its_owner_being_replaced() {
    let h = Harness::new();
    h.differential_row("rc2-28", RC2_28, &[], "111\n", (7, 7));
    h.lsan_row("rc2-28", RC2_28, &[], "111\n");
    h.assert_legs(24);
}

const RC2_29: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
fn zero(h: Rc<W>) -> i64 { return 0 }
struct W { f: fn(Rc<W>) -> i64 }
fn run(h: Rc<W>, f: fn(Rc<W>) -> i64) -> i64 { return f(h) }
fn main() -> i64 {
    let s: string = victim(12345)
    let a = Rc::new(W { f: fn (h: Rc<W>) -> i64 {
        h.f = zero
        let t: string = other(88888)
        return s.len + s.bytes[0] + t.len
    } })
    println("{run(a, a.f)}")
    return 0
}
"#;

#[test]
fn rc2_29_a_closure_field_passed_as_argument_survives_replacing_itself_through_the_handle() {
    let h = Harness::new();
    h.differential_row("rc2-29", RC2_29, &[], "111\n", (6, 6));
    h.lsan_row("rc2-29", RC2_29, &[], "111\n");
    h.assert_legs(24);
}

const RC2_30: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct In { s: string }
struct W { s: string, a: [string; 2], i: In }
fn clob(h: Rc<W>) -> string {
    h.s = victim(1)
    h.a[1] = victim(1)
    h.i.s = victim(1)
    let t: string = other(88888)
    return ""
}
fn mk() -> Rc<W> {
    return Rc::new(W { s: victim(12345), a: [victim(1), victim(12345)], i: In { s: victim(12345) } })
}
fn main() -> i64 {
    let a = mk()
    a.s += clob(a)
    let b = mk()
    b.s = b.s + clob(b)
    let t: string = other(77777)
    println("{a.s.bytes[0] + a.s.len + t.len} {b.s.bytes[0] + b.s.len + t.len}")
    return 0
}
"#;

#[test]
fn rc2_30_a_compound_field_append_keeps_the_old_value_while_the_callee_writes_the_handle() {
    let h = Harness::new();
    h.differential_row("rc2-30", RC2_30, &[], "111 111\n", (42, 42));
    h.lsan_row("rc2-30", RC2_30, &[], "111 111\n");
    h.assert_legs(24);
}

const RC2_31: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct In { s: string }
struct W { s: string, a: [string; 2], i: In }
fn clob(h: Rc<W>) -> string {
    h.s = victim(1)
    h.a[1] = victim(1)
    h.i.s = victim(1)
    let t: string = other(88888)
    return ""
}
fn mk() -> Rc<W> {
    return Rc::new(W { s: victim(12345), a: [victim(1), victim(12345)], i: In { s: victim(12345) } })
}
fn main() -> i64 {
    let a = mk()
    a.a[1] += clob(a)
    let b = mk()
    b.a[1] = b.a[1] + clob(b)
    let t: string = other(77777)
    println("{a.a[1].bytes[0] + a.a[1].len + t.len} {b.a[1].bytes[0] + b.a[1].len + t.len}")
    return 0
}
"#;

#[test]
fn rc2_31_a_compound_element_append_keeps_the_old_value_while_the_callee_writes_the_handle() {
    let h = Harness::new();
    h.differential_row("rc2-31", RC2_31, &[], "111 111\n", (42, 42));
    h.lsan_row("rc2-31", RC2_31, &[], "111 111\n");
    h.assert_legs(24);
}

const RC2_32: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct In { s: string }
struct W { s: string, a: [string; 2], i: In }
fn clob(h: Rc<W>) -> string {
    h.s = victim(1)
    h.a[1] = victim(1)
    h.i.s = victim(1)
    let t: string = other(88888)
    return ""
}
fn mk() -> Rc<W> {
    return Rc::new(W { s: victim(12345), a: [victim(1), victim(12345)], i: In { s: victim(12345) } })
}
fn main() -> i64 {
    let a = mk()
    a.i.s += clob(a)
    let b = mk()
    b.i.s = b.i.s + clob(b)
    let t: string = other(77777)
    println("{a.i.s.bytes[0] + a.i.s.len + t.len} {b.i.s.bytes[0] + b.i.s.len + t.len}")
    return 0
}
"#;

#[test]
fn rc2_32_a_compound_nested_append_keeps_the_old_value_while_the_callee_writes_the_handle() {
    let h = Harness::new();
    h.differential_row("rc2-32", RC2_32, &[], "111 111\n", (42, 42));
    h.lsan_row("rc2-32", RC2_32, &[], "111 111\n");
    h.assert_legs(24);
}

const RC2_33: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn clobber(h: Rc<W>, bs: &[u8]) -> i64 {
    h.s = victim(1)
    let t: string = other(88888)
    return bs[0] + bs.len + t.len
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    println("{clobber(a, Rc::get(a).s.bytes)}")
    return 0
}
"#;

#[test]
fn rc2_33_a_bytes_view_argument_of_a_payload_field_survives_the_callee_replacing_the_field() {
    let h = Harness::new();
    h.differential_row("rc2-33", RC2_33, &[], "111\n", (7, 7));
    h.lsan_row("rc2-33", RC2_33, &[], "111\n");
    h.assert_legs(24);
}

const RC2_34: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn w(h: Rc<W>) -> i64 {
    h.s = victim(1)
    return 0
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    let mut n: i64 = 0
    for b in Rc::get(a).s.bytes {
        let z = w(a)
        let t: string = other(88888)
        n = n + b + z
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn rc2_34_a_for_over_the_bytes_of_a_payload_field_survives_the_body_replacing_the_field() {
    let h = Harness::new();
    h.differential_row("rc2-34", RC2_34, &[], "450\n", (31, 31));
    h.lsan_row("rc2-34", RC2_34, &[], "450\n");
    h.assert_legs(24);
}

const RC2_35: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }
fn w(h: Rc<W>) -> i64 {
    h.s = victim(1)
    return 0
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    let b = a
    let bs = bytes_of(&Rc::get(a))
    b.s = victim(1)
    let cs = bytes_of(&Rc::get(a))
    let z = w(a)
    let t: string = other(88888)
    println("{bs[0] + bs.len + t.len} {cs[0] + cs.len + z}")
    return 0
}
"#;

#[test]
fn rc2_35_a_bound_view_returned_by_a_call_survives_a_store_through_the_handle() {
    let h = Harness::new();
    h.differential_row("rc2-35", RC2_35, &[], "111 100\n", (13, 13));
    h.lsan_row("rc2-35", RC2_35, &[], "111 100\n");
    h.assert_legs(24);
}

const RC2_36: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn g(k: i64) -> i64 { return k }
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    let b = a
    let k: string = other(5)
    let bs = if g(1) == 1 { Rc::get(a).s.bytes } else { k.bytes }
    b.s = victim(1)
    let t: string = other(88888)
    println("{bs[0] + bs.len + t.len}")
    return 0
}
"#;

#[test]
fn rc2_36_a_view_chosen_by_an_if_survives_a_store_through_an_alias() {
    let h = Harness::new();
    h.differential_row("rc2-36", RC2_36, &[], "111\n", (9, 9));
    h.lsan_row("rc2-36", RC2_36, &[], "111\n");
    h.assert_legs(24);
}

const RC2_37: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { a: [string; 2] }
fn clobber(h: Rc<W>, s: string) -> i64 {
    h.a[1] = victim(1)
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn main() -> i64 {
    let a = Rc::new(W { a: [victim(1), victim(12345)] })
    let v = Rc::get(a).a[0..2]
    println("{clobber(a, v[1])}")
    return 0
}
"#;

#[test]
fn rc2_37_a_slice_element_into_a_payload_passed_as_argument_survives_the_callee_replacing_it() {
    let h = Harness::new();
    h.differential_row("rc2-37", RC2_37, &[], "111\n", (9, 9));
    h.lsan_row("rc2-37", RC2_37, &[], "111\n");
    h.assert_legs(24);
}

const RC2_38: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { a: [string; 2] }
fn main() -> i64 {
    let a = Rc::new(W { a: [victim(1), victim(12345)] })
    let b = a
    let v = Rc::get(a).a[0..2]
    let bs = v[1].bytes
    b.a[1] = victim(1)
    let t: string = other(88888)
    println("{bs[0] + bs.len + t.len}")
    return 0
}
"#;

#[test]
fn rc2_38_a_bytes_view_of_a_slice_element_into_a_payload_survives_a_store_through_an_alias() {
    let h = Harness::new();
    h.differential_row("rc2-38", RC2_38, &[], "111\n", (9, 9));
    h.lsan_row("rc2-38", RC2_38, &[], "111\n");
    h.assert_legs(24);
}

const RC2_39: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn clobber(h: Rc<W>, e: string) -> i64 {
    h.s = victim(1)
    let t: string = other(88888)
    return e.bytes[0] + e.len + t.len
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    let b = Rc::new(W { s: victim(12345) })
    println("{clobber(a, "{a.s}")} {clobber(b, "{Rc::get(b).s}")}")
    return 0
}
"#;

#[test]
fn rc2_39_a_single_part_interpolation_argument_survives_the_callee_replacing_the_field() {
    let h = Harness::new();
    h.differential_row("rc2-39", RC2_39, &[], "111 111\n", (18, 18));
    h.lsan_row("rc2-39", RC2_39, &[], "111 111\n");
    h.assert_legs(24);
}

const RC2_40: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
enum E { A(string), B }
struct W { e: E }
fn clobber(h: Rc<W>, e: E) -> i64 {
    h.e = E::A(victim(1))
    let t: string = other(88888)
    return match e { E::A(x) => x.bytes[0] + x.len + t.len, E::B => 0 }
}
fn main() -> i64 {
    let a = Rc::new(W { e: E::A(victim(12345)) })
    println("{clobber(a, a.e)}")
    return 0
}
"#;

#[test]
fn rc2_40_an_enum_field_argument_survives_the_callee_replacing_the_field() {
    let h = Harness::new();
    h.differential_row("rc2-40", RC2_40, &[], "111\n", (7, 7));
    h.lsan_row("rc2-40", RC2_40, &[], "111\n");
    h.assert_legs(24);
}

const RC2_41: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { a: [string; 2] }
fn clobber(h: Rc<W>, e: [string; 2]) -> i64 {
    h.a = [victim(1), victim(2)]
    let t: string = other(88888)
    return e[1].bytes[0] + e[1].len + t.len
}
fn main() -> i64 {
    let a = Rc::new(W { a: [victim(1), victim(12345)] })
    println("{clobber(a, a.a)}")
    return 0
}
"#;

#[test]
fn rc2_41_an_array_field_argument_survives_the_callee_replacing_the_field() {
    let h = Harness::new();
    h.differential_row("rc2-41", RC2_41, &[], "111\n", (11, 11));
    h.lsan_row("rc2-41", RC2_41, &[], "111\n");
    h.assert_legs(24);
}

const RC2_42: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }
fn w(h: Rc<W>) -> i64 {
    h.s = victim(1)
    return 0
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    let mut n: i64 = 0
    for b in bytes_of(&Rc::get(a)) {
        let z = w(a)
        let t: string = other(88888)
        n = n + b + z
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn rc2_42_a_for_over_a_view_returned_by_a_call_survives_the_body_replacing_the_field() {
    let h = Harness::new();
    h.differential_row("rc2-42", RC2_42, &[], "450\n", (31, 31));
    h.lsan_row("rc2-42", RC2_42, &[], "450\n");
    h.assert_legs(24);
}

const RC2_43: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn bytes_of(w: &W) -> &[u8] { return (*w).s.bytes }
fn w(h: Rc<W>) -> i64 {
    h.s = victim(1)
    return 0
}
fn clob(h: Rc<W>, bs: &[u8]) -> i64 {
    let z = w(h)
    let t: string = other(88888)
    return bs[0] + bs.len + t.len + z
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    println("{clob(a, bytes_of(&Rc::get(a)))}")
    return 0
}
"#;

#[test]
fn rc2_43_a_view_argument_returned_by_a_call_survives_the_callee_replacing_the_field() {
    let h = Harness::new();
    h.differential_row("rc2-43", RC2_43, &[], "111\n", (7, 7));
    h.lsan_row("rc2-43", RC2_43, &[], "111\n");
    h.assert_legs(24);
}

const RC2_44: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
enum E { A(string), B }
struct In { s: string }
struct W { s: string, a: [string; 2], i: In, e: E }
fn len_of(e: E) -> i64 { return match e { E::A(x) => x.bytes[0] + x.len, E::B => 0 } }
fn cs(h: Rc<W>, s: string) -> i64 {
    Rc::get(h).s = victim(1)
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn ci(h: Rc<W>, s: string) -> i64 {
    Rc::get(h).i.s = victim(1)
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn ca(h: Rc<W>, s: string) -> i64 {
    Rc::get(h).a[1] = victim(1)
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn ce(h: Rc<W>, e: E) -> i64 {
    Rc::get(h).e = E::A(victim(1))
    let t: string = other(88888)
    return len_of(e) + t.len
}
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345), a: [victim(1), victim(12345)], i: In { s: victim(12345) }, e: E::A(victim(12345)) })
    println("{cs(a, a.s)} {ci(a, a.i.s)} {ca(a, Rc::get(a).a[1])} {ce(a, a.e)}")
    println("{a.s} {a.i.s} {a.a[1]}")
    return 0
}
"#;

#[test]
fn rc2_44_a_field_written_through_rc_get_is_released_while_its_argument_reader_holds_it() {
    let h = Harness::new();
    h.differential_row(
        "rc2-44",
        RC2_44,
        &[],
        "111 111 111 111\nab1 ab1 ab1\n",
        (41, 41),
    );
    h.lsan_row("rc2-44", RC2_44, &[], "111 111 111 111\nab1 ab1 ab1\n");
    h.assert_legs(24);
}

const RC2_45: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct V { s: string }
struct In { s: string }
struct W { a: [string; 2], i: In, nx: Rc<V> }
fn cw(h: Rc<W>, s: string) -> i64 {
    Rc::get(h).a = [victim(1), victim(2)]
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn ci(h: Rc<W>, s: string) -> i64 {
    Rc::get(h).i = In { s: victim(1) }
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn cn(h: Rc<W>, s: string) -> i64 {
    Rc::get(h.nx).s = victim(1)
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn cm(h: Rc<W>, s: string) -> i64 {
    Rc::get(Rc::get(h).nx).s = victim(12)
    let t: string = other(88888)
    return s.bytes[0] + s.len + t.len
}
fn main() -> i64 {
    let a = Rc::new(W { a: [victim(1), victim(12345)], i: In { s: victim(12345) }, nx: Rc::new(V { s: victim(12345) }) })
    println("{cw(a, a.a[1])} {ci(a, a.i.s)} {cn(a, a.nx.s)} {cm(a, Rc::get(a.nx).s)}")
    println("{a.a[1]} {a.i.s} {a.nx.s}")
    return 0
}
"#;

#[test]
fn rc2_45_a_whole_field_or_inner_rc_written_through_rc_get_is_released_after_its_argument() {
    let h = Harness::new();
    h.differential_row(
        "rc2-45",
        RC2_45,
        &[],
        "111 111 111 107\nab2 ab1 ab12\n",
        (42, 42),
    );
    h.lsan_row("rc2-45", RC2_45, &[], "111 111 111 107\nab2 ab1 ab12\n");
    h.assert_legs(24);
}

const RC2_46: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct In { s: string }
struct W { s: string, i: In }
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345), i: In { s: victim(12345) } })
    let b = a
    let bs = a.s.bytes
    Rc::get(b).s = victim(1)
    let cs = Rc::get(a).i.s.bytes
    Rc::get(b).i.s += victim(7)
    let t: string = other(88888)
    println("{bs[0] + bs.len + t.len} {cs[0] + cs.len} {a.s} {a.i.s}")
    return 0
}
"#;

#[test]
fn rc2_46_a_field_written_through_rc_get_is_released_while_a_bound_view_holds_it() {
    let h = Harness::new();
    h.differential_row("rc2-46", RC2_46, &[], "111 104 ab1 ab12345ab7\n", (20, 20));
    h.lsan_row("rc2-46", RC2_46, &[], "111 104 ab1 ab12345ab7\n");
    h.assert_legs(24);
}

const RC2_47: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct In { s: string }
struct W { i: In }
fn main() -> i64 {
    let a = Rc::new(W { i: In { s: victim(12345) } })
    let mut n: i64 = 0
    for b in Rc::get(a).i.s.bytes {
        Rc::get(a).i.s = victim(n)
        let t: string = other(88888)
        n = n + b
    }
    println("{n} {a.i.s}")
    return 0
}
"#;

#[test]
fn rc2_47_a_field_written_through_rc_get_in_a_for_body_is_released_after_the_loop() {
    let h = Harness::new();
    h.differential_row("rc2-47", RC2_47, &[], "450 ab397\n", (34, 34));
    h.lsan_row("rc2-47", RC2_47, &[], "450 ab397\n");
    h.assert_legs(24);
}

const RC2_48: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct V { s: string }
struct W { s: string, nx: Rc<V> }
fn main() -> i64 {
    let a = Rc::new(W { s: victim(1), nx: Rc::new(V { s: victim(4) }) })
    let w = W { s: victim(9), nx: Rc::new(V { s: victim(12345) }) }
    let mut n: i64 = 0
    while n < 3 {
        let r = &mut Rc::get(a)
        *r = W { s: victim(n), nx: Rc::new(V { s: victim(n) }) }
        let q = &mut Rc::get(a)
        *q = w
        n = n + 1
    }
    println("{a.s} {a.nx.s} {w.s} {w.nx.s}")
    return 0
}
"#;

#[test]
fn rc2_48_a_whole_payload_written_through_a_mutable_reference_releases_its_old_rc_leaves() {
    let h = Harness::new();
    h.differential_row("rc2-48", RC2_48, &[], "ab9 ab12345 ab9 ab12345\n", (32, 32));
    h.lsan_row("rc2-48", RC2_48, &[], "ab9 ab12345 ab9 ab12345\n");
    h.assert_legs(24);
}

const RC2_49: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct V { s: string }
struct W { nx: Rc<V> }
fn main() -> i64 {
    let a = Rc::new(W { nx: Rc::new(V { s: victim(4) }) })
    let w = W { nx: Rc::new(V { s: victim(12345) }) }
    let mut n: i64 = 0
    while n < 3 {
        let r = &mut Rc::get(a)
        *r = w
        let q = &mut Rc::get(a)
        *q = W { nx: Rc::new(V { s: victim(n) }) }
        n = n + 1
    }
    println("{a.nx.s} {w.nx.s}")
    return 0
}
"#;

#[test]
fn rc2_49_a_payload_of_only_rc_leaves_copied_through_a_mutable_reference_takes_its_share() {
    let h = Harness::new();
    h.differential_row("rc2-49", RC2_49, &[], "ab2 ab12345\n", (18, 18));
    h.lsan_row("rc2-49", RC2_49, &[], "ab2 ab12345\n");
    h.assert_legs(24);
}

fn refusal_row(id: &str, src: &str, files: &[(&str, &str)], code: &str, needle: &str) {
    refusal_row_full(id, src, files, code, &[needle], &[]);
}

fn refusal_row_full(
    id: &str,
    src: &str,
    files: &[(&str, &str)],
    code: &str,
    needles: &[&str],
    absent: &[&str],
) {
    let h = Harness::new();
    for (tag, opt) in LEVELS {
        let root = h.stage(id, &format!("{tag}_refuse"), src, files);
        let err = compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::Rc)
            .expect_err(&format!("{id} at {tag} compiled"));
        let text = err.to_string();
        assert!(
            text.contains(&format!("[{code}]")),
            "{id} at {tag}: code\n{text}"
        );
        assert!(
            !text.contains("E0432"),
            "{id} at {tag}: verdict split\n{text}"
        );
        for needle in needles {
            assert!(text.contains(needle), "{id} at {tag}: diagnostic\n{text}");
        }
        for gone in absent {
            assert!(!text.contains(gone), "{id} at {tag}: stale text\n{text}");
        }
    }
}

const RC2_50: &str = r#"
fn boxit<T>(x: T) -> i64 {
    let r: Rc<T> = Rc::new(x)
    return 1
}
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(41)
    let n: i64 = boxit(a)
    let b: Rc<i64> = Rc::new(1000)
    println("{Rc::get(a) + n}")
    return 0
}
"#;

#[test]
fn rc2_50_an_rc_boxed_by_a_generic_takes_its_share_for_the_new_payload() {
    let h = Harness::new();
    h.differential_row("rc2-50", RC2_50, &[], "42\n", (3, 3));
    h.lsan_row("rc2-50", RC2_50, &[], "42\n");
    h.assert_legs(24);
}

const RC2_51: &str = r#"
fn wrap<T>(x: T) -> Rc<T> { return Rc::new(x) }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(41)
    let r: Rc<Rc<i64>> = wrap(a)
    let b: Rc<i64> = Rc::new(1000)
    println("{Rc::get(a)}")
    return 0
}
"#;

#[test]
fn rc2_51_an_rc_wrapped_and_returned_by_a_generic_is_released_once() {
    let h = Harness::new();
    h.differential_row("rc2-51", RC2_51, &[], "41\n", (3, 3));
    h.lsan_row("rc2-51", RC2_51, &[], "41\n");
    h.assert_legs(24);
}

const RC2_52: &str = r#"
fn boxit<T>(mut x: T, y: T) -> i64 {
    let z: T = y
    x = z
    let r: Rc<T> = Rc::new(x)
    return 1
}
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(41)
    let c: Rc<i64> = Rc::new(5)
    let n: i64 = boxit(c, a)
    let b: Rc<i64> = Rc::new(1000)
    println("{Rc::get(a) + Rc::get(c) + n}")
    return 0
}
"#;

#[test]
fn rc2_52_a_generic_parameter_reassigned_from_another_rc_keeps_both_shares() {
    let h = Harness::new();
    h.differential_row("rc2-52", RC2_52, &[], "47\n", (4, 4));
    h.lsan_row("rc2-52", RC2_52, &[], "47\n");
    h.assert_legs(24);
}

const RC2_53: &str = r#"
enum One<T> { It(T) }
struct W { r: Rc<i64> }
fn boxone<T>(x: T) -> i64 {
    let r: Rc<One<T>> = Rc::new(One::It(x))
    return 1
}
fn main() -> i64 {
    let w: W = W { r: Rc::new(41) }
    let n: i64 = boxone(w)
    let b: Rc<i64> = Rc::new(1000)
    println("{Rc::get(w.r) + n}")
    return 0
}
"#;

#[test]
fn rc2_53_an_rc_leaf_under_a_generic_enum_payload_is_released_with_the_payload() {
    let h = Harness::new();
    h.differential_row("rc2-53", RC2_53, &[], "42\n", (3, 3));
    h.lsan_row("rc2-53", RC2_53, &[], "42\n");
    h.assert_legs(24);
}

const RC2_54: &str = r#"
enum One<T> { It(T) }
fn f_match<T>(x: T) -> T {
    let o: One<T> = One::It(x)
    return match o { One::It(p) => p }
}
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(41)
    let v: Rc<i64> = f_match(a)
    let z: Rc<i64> = Rc::new(1000)
    println("{Rc::get(v) + Rc::get(a)}")
    return 0
}
"#;

#[test]
fn rc2_54_an_rc_through_a_single_variant_generic_enum_comes_back_with_its_share() {
    let h = Harness::new();
    h.differential_row("rc2-54", RC2_54, &[], "82\n", (2, 2));
    h.lsan_row("rc2-54", RC2_54, &[], "82\n");
    h.assert_legs(24);
}

const RC2_55_LIB: &str = r#"
pub fn boxit<T>(x: T) -> i64 {
    let r: Rc<T> = Rc::new(x)
    return 1
}
"#;

const RC2_55: &str = r#"
needs lib1
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(41)
    let n: i64 = lib1.boxit(a)
    let b: Rc<i64> = Rc::new(1000)
    println("{Rc::get(a) + n}")
    return 0
}
"#;

#[test]
fn rc2_55_an_imported_generic_boxing_an_rc_takes_its_share() {
    let h = Harness::new();
    let files = [("lib1.aelys", RC2_55_LIB)];
    h.differential_row("rc2-55", RC2_55, &files, "42\n", (3, 3));
    h.lsan_row("rc2-55", RC2_55, &files, "42\n");
    h.assert_legs(24);
}

#[test]
fn rc2_56_a_reference_boxed_by_a_generic_is_refused_at_the_call() {
    refusal_row(
        "rc2-56",
        "fn boxit<T>(x: T) -> i64 {\n    let r: Rc<T> = Rc::new(x)\n    return 1\n}\n\
         fn main() -> i64 {\n    let a: [i64; 3] = [7, 0, 41]\n    let n: i64 = boxit(&a[2])\n\
         \x20   println(\"{a[0]} {a[2] + n}\")\n    return 0\n}\n",
        &[],
        "E0714",
        "the call to `boxit` in `main` instantiates it with `&i64`, which makes a `Rc<&i64>`",
    );
}

#[test]
fn rc2_57_a_reference_wrapped_in_an_rc_by_a_generic_in_a_loop_is_refused() {
    refusal_row(
        "rc2-57",
        "fn wrap<T>(x: T) -> Rc<T> { return Rc::new(x) }\nfn main() -> i64 {\n\
         \x20   let s: [i64; 3] = [1, 0, 41]\n    let mut i: i64 = 0\n    while i < 2 {\n\
         \x20       let r: Rc<&i64> = wrap(&s[2])\n        i = i + 1\n    }\n\
         \x20   println(\"{s[0]}\")\n    return 0\n}\n",
        &[],
        "E0714",
        "the call to `wrap` in `main`",
    );
}

#[test]
fn rc2_58_a_stack_reference_escaping_in_a_generic_enum_is_refused() {
    refusal_row(
        "rc2-58",
        "enum One<T> { It(T) }\nfn wrapone<T>(x: T) -> One<T> { return One::It(x) }\n\
         fn mk() -> One<&i64> {\n    let v: [i64; 3] = [7, 8, 9]\n    return wrapone(&v[1])\n}\n\
         fn main() -> i64 {\n    let o: One<&i64> = mk()\n\
         \x20   let n: i64 = match o { One::It(p) => *p }\n    println(\"{n}\")\n    return 0\n}\n",
        &[],
        "E0714",
        "the call to `wrapone` in `mk` instantiates it with `&i64`, which makes a `One<&i64>`",
    );
}

#[test]
fn rc2_59_a_stack_reference_escaping_in_an_rc_of_a_generic_enum_is_refused() {
    refusal_row(
        "rc2-59",
        "enum One<T> { It(T) }\nfn wrapone<T>(x: T) -> Rc<One<T>> { return Rc::new(One::It(x)) }\n\
         fn mk() -> Rc<One<&i64>> {\n    let v: [i64; 3] = [7, 8, 9]\n    return wrapone(&v[1])\n}\n\
         fn main() -> i64 {\n    let r: Rc<One<&i64>> = mk()\n    return 0\n}\n",
        &[],
        "E0714",
        "which makes a `Rc<One<&i64>>`",
    );
}

#[test]
fn rc2_60_an_rc_placed_in_an_array_by_a_generic_is_refused() {
    refusal_row(
        "rc2-60",
        "fn f_arr<T>(a: T, b: T) -> T {\n    let arr: [T; 2] = [a, b]\n    return arr[0]\n}\n\
         fn main() -> i64 {\n    let a: Rc<i64> = Rc::new(41)\n    let b: Rc<i64> = Rc::new(2)\n\
         \x20   let v: Rc<i64> = f_arr(a, b)\n    println(\"{Rc::get(v) + Rc::get(a)}\")\n\
         \x20   return 0\n}\n",
        &[],
        "E0410",
        "the call to `f_arr` in `main` instantiates it with `Rc<i64>`, which makes a `[Rc<i64>; 2]`",
    );
}

#[test]
fn rc2_61_an_rc_carrier_placed_in_an_array_inside_an_rc_by_a_generic_is_refused() {
    refusal_row(
        "rc2-61",
        "struct W { r: Rc<i64> }\nfn boxarr<T>(x: T) -> i64 {\n\
         \x20   let r: Rc<[T; 2]> = Rc::new([x, x])\n    return 1\n}\n\
         fn main() -> i64 {\n    let w: W = W { r: Rc::new(41) }\n    let n: i64 = boxarr(w)\n\
         \x20   println(\"{Rc::get(w.r) + n}\")\n    return 0\n}\n",
        &[],
        "E0410",
        "which makes a `[W; 2]`",
    );
}

#[test]
fn rc2_62_an_imported_generic_boxing_a_reference_is_refused() {
    refusal_row(
        "rc2-62",
        "needs lib1\nfn main() -> i64 {\n    let s: [i64; 3] = [7, 0, 41]\n\
         \x20   let n: i64 = lib1.boxit(&s[2])\n    println(\"{s[0]} {n}\")\n    return 0\n}\n",
        &[("lib1.aelys", RC2_55_LIB)],
        "E0714",
        "the call to `lib1.boxit` in `main`",
    );
}

const RC2_63: &str = r#"
struct N { s: string }
struct W { r: Rc<N>, k: i64 }
enum May<T> { Yes(T), No }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn wrap<T>(x: T) -> May<T> { return May::Yes(x) }
fn main() -> i64 {
    let mut i = 0
    let mut t = 0
    while i < 1000 { let r = Rc::new(N { s: victim(i) })
      let m = wrap(W { r: r, k: i })
      match m { May::Yes(w) => { t = t + w.k } May::No => { t = t } }
      i = i + 1 }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_63_an_rc_carrier_in_a_multi_variant_generic_enum_is_refused() {
    refusal_row("rc2-63", RC2_63, &[], "E0410", "which makes a `May<W>`");
}

#[test]
fn rc2_64_an_rc_through_a_single_variant_enum_in_a_multi_variant_enum_is_refused() {
    refusal_row(
        "rc2-64",
        "struct N { s: string }\nenum One { It(Rc<N>) }\nenum L { A(One), B(i64) }\n\
         fn go(l: L) -> i64 { match l { L::A(o) => { match o { One::It(r) => { return Rc::get(r).s.len } } } L::B(k) => { return k } } }\n\
         fn main() -> i64 {\n    let r = Rc::new(N { s: \"ab\" })\n\
         \x20   println(\"{go(L::A(One::It(r)))}\")\n    return 0\n}\n",
        &[],
        "E0410",
        "variant `L::A` has a payload of type `One`, which holds an `Rc`: an enum with more than \
         one variant cannot carry an Rc yet",
    );
}

const RC2_65: &str = r#"
struct N { v: i64 }
enum One<T> { It(T) }
fn mk1<T>(x: T) -> One<T> { return One::It(x) }
fn use1<T>(o: One<T>) -> i64 { return 1 }
fn main() -> i64 {
    let mut s: i64 = 0
    let mut i: i64 = 0
    while i < 1000 {
        let p: Rc<N> = Rc::new(N { v: i })
        s = s + use1(mk1(p))
        i = i + 1
    }
    println("{s}")
    return 0
}
"#;

#[test]
fn rc2_65_a_generic_enum_instance_passed_as_an_argument_releases_its_rc() {
    let h = Harness::new();
    h.differential_row("rc2-65", RC2_65, &[], "1000\n", (1000, 1000));
    h.lsan_row("rc2-65", RC2_65, &[], "1000\n");
    h.assert_legs(24);
}

const RC2_66: &str = r#"
struct N { v: i64 }
enum One<T> { It(T) }
fn mk1<T>(x: T) -> One<T> { return One::It(x) }
fn get1<T>(o: One<T>) -> T {
    return match o {
        One::It(v) => v
    }
}
fn main() -> i64 {
    let mut s: i64 = 0
    let mut i: i64 = 0
    while i < 1000 {
        let q: Rc<N> = get1(mk1(Rc::new(N { v: i })))
        s = s + Rc::get(q).v
        i = i + 1
    }
    println("{s}")
    return 0
}
"#;

#[test]
fn rc2_66_a_generic_enum_instance_unwrapped_by_a_generic_releases_its_rc() {
    let h = Harness::new();
    h.differential_row("rc2-66", RC2_66, &[], "499500\n", (1000, 1000));
    h.lsan_row("rc2-66", RC2_66, &[], "499500\n");
    h.assert_legs(24);
}

const RC2_67: &str = r#"
struct N { v: i64 }
enum One<T> { It(T) }
fn mk1<T>(x: T) -> One<T> { return One::It(x) }
fn use1<T>(o: One<T>) -> i64 { return 1 }
fn main() -> i64 {
    let mut s: i64 = 0
    let mut i: i64 = 0
    while i < 1000 {
        let p: Rc<N> = Rc::new(N { v: i })
        let o = mk1(p)
        s = s + use1(o)
        i = i + 1
    }
    println("{s}")
    return 0
}
"#;

#[test]
fn rc2_67_a_generic_enum_instance_bound_by_let_releases_its_rc() {
    let h = Harness::new();
    h.differential_row("rc2-67", RC2_67, &[], "1000\n", (1000, 1000));
    h.lsan_row("rc2-67", RC2_67, &[], "1000\n");
    h.assert_legs(24);
}

const RC2_68: &str = r#"
struct N { v: i64 }
enum One<T> { It(T) }
fn mk1<T>(x: T) -> One<T> { return One::It(x) }
fn use1<T>(o: One<T>) -> i64 { return 1 }
fn go<T>(x: T) -> i64 { return use1(mk1(x)) }
fn main() -> i64 {
    let mut s: i64 = 0
    let mut i: i64 = 0
    while i < 1000 {
        let p: Rc<N> = Rc::new(N { v: i })
        s = s + go(p)
        i = i + 1
    }
    println("{s}")
    return 0
}
"#;

#[test]
fn rc2_68_a_generic_enum_instance_made_inside_a_generic_releases_its_rc() {
    let h = Harness::new();
    h.differential_row("rc2-68", RC2_68, &[], "1000\n", (1000, 1000));
    h.lsan_row("rc2-68", RC2_68, &[], "1000\n");
    h.assert_legs(24);
}

const RC2_69: &str = r#"
struct N { v: i64 }
enum One<T> { It(T) }
struct H { o: One<Rc<N>>, k: i64 }
fn mk1<T>(x: T) -> One<T> { return One::It(x) }
fn get1<T>(o: One<T>) -> T { return match o { One::It(v) => v } }
fn main() -> i64 {
    let mut s: i64 = 0
    let mut i: i64 = 0
    while i < 1000 {
        let p: Rc<N> = Rc::new(N { v: i })
        mk1(p)
        let k: i64 = match mk1(p) { One::It(q) => { let qq: Rc<N> = q
            Rc::get(qq).v } }
        let o = mk1(p)
        let h: H = H { o: o, k: 2 }
        let r: Rc<N> = get1(h.o)
        s = s + k + h.k + Rc::get(r).v
        i = i + 1
    }
    println("{s}")
    return 0
}
"#;

#[test]
fn rc2_69_a_generic_enum_instance_discarded_matched_or_stored_in_a_field_releases_its_rc() {
    let h = Harness::new();
    h.differential_row("rc2-69", RC2_69, &[], "1001000\n", (1000, 1000));
    h.lsan_row("rc2-69", RC2_69, &[], "1001000\n");
    h.assert_legs(24);
}

#[test]
fn rc2_70_an_rc_pushed_into_a_vec_by_a_generic_is_refused() {
    refusal_row(
        "rc2-70",
        "struct N { v: i64 }\nfn mkv<T>(x: T) -> i64 {\n    let mut v: Vec<T> = Vec::new()\n\
         \x20   Vec::push(v, x)\n    Vec::push(v, x)\n    return Vec::len(v)\n}\n\
         fn main() -> i64 {\n    let p: Rc<N> = Rc::new(N { v: 9 })\n    let n: i64 = mkv(p)\n\
         \x20   println(\"{n} {Rc::get(p).v}\")\n    return 0\n}\n",
        &[],
        "E0410",
        "the call to `mkv` in `main` instantiates it with `Rc<N>`, which makes a `Vec<Rc<N>>`: a \
         `Vec` does not release the `Rc` it holds yet",
    );
}

#[test]
fn rc2_71_an_rc_pushed_into_a_vec_by_a_generic_in_a_loop_is_refused() {
    refusal_row(
        "rc2-71",
        "struct N { v: i64 }\nfn mkv<T>(x: T) -> i64 {\n    let mut v: Vec<T> = Vec::new()\n\
         \x20   Vec::push(v, x)\n    return Vec::len(v)\n}\nfn main() -> i64 {\n\
         \x20   let mut s: i64 = 0\n    let mut i: i64 = 0\n    while i < 1000 {\n\
         \x20       let p: Rc<N> = Rc::new(N { v: i })\n        s = s + mkv(p)\n        i = i + 1\n\
         \x20   }\n    println(\"{s}\")\n    return 0\n}\n",
        &[],
        "E0410",
        "which makes a `Vec<Rc<N>>`",
    );
}

#[test]
fn rc2_72_an_rc_in_a_vec_copied_by_a_generic_is_refused() {
    refusal_row(
        "rc2-72",
        "struct N { v: i64 }\nfn two<T>(x: T) -> i64 {\n    let mut v: Vec<T> = Vec::new()\n\
         \x20   Vec::push(v, x)\n    let w: Vec<T> = v\n    Vec::push(w, x)\n\
         \x20   return Vec::len(v) + Vec::len(w)\n}\nfn main() -> i64 {\n\
         \x20   let p: Rc<N> = Rc::new(N { v: 9 })\n    let n: i64 = two(p)\n\
         \x20   println(\"{n} {Rc::get(p).v}\")\n    return 0\n}\n",
        &[],
        "E0410",
        "the call to `two` in `main` instantiates it with `Rc<N>`, which makes a `Vec<Rc<N>>`",
    );
}

#[test]
fn rc2_73_a_byte_view_put_in_a_generic_enum_is_refused() {
    refusal_row(
        "rc2-73",
        "enum One<T> { It(T) }\nfn mk1<T>(x: T) -> One<T> { return One::It(x) }\n\
         fn first(o: One<&[u8]>) -> u8 {\n    return match o {\n        One::It(b) => b[0]\n    }\n}\n\
         fn main() -> i64 {\n    let mut s: string = \"ab\" + \"cd\"\n    let o = mk1(s.bytes)\n\
         \x20   s = \"zz\" + \"yy\"\n    let c: u8 = first(o)\n    println(\"{c} {s}\")\n\
         \x20   return 0\n}\n",
        &[],
        "E0714",
        "the call to `mk1` in `main` instantiates it with `&[u8]`, which makes a `One<&[u8]>`",
    );
}

#[test]
fn rc2_74_a_byte_view_put_in_an_rc_by_a_generic_is_refused() {
    refusal_row(
        "rc2-74",
        "fn wrap<T>(x: T) -> Rc<T> { return Rc::new(x) }\n\
         fn first<T>(r: Rc<T>) -> T { return Rc::get(r) }\n\
         fn byte0(b: &[u8]) -> u8 { return b[0] }\nfn main() -> i64 {\n\
         \x20   let mut s: string = \"ab\" + \"cd\"\n    let r = wrap(s.bytes)\n\
         \x20   s = \"zz\" + \"yy\"\n    let c: u8 = byte0(first(r))\n    println(\"{c} {s}\")\n\
         \x20   return 0\n}\n",
        &[],
        "E0714",
        "the call to `wrap` in `main` instantiates it with `&[u8]`, which makes a `Rc<&[u8]>`",
    );
}

const RC2_75_BURN: &str = "fn burn() -> i64 {\n\
    \x20   let z: [i64; 16] = [5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5]\n    return z[3]\n}\n\
    fn main() -> i64 {\n    let v = esc(1)\n    let b: i64 = burn()\n    println(\"{*v[0]} {b}\")\n\
    \x20   return 0\n}\n";

#[test]
fn rc2_75_a_reference_escaping_in_a_vec_made_by_a_generic_is_refused() {
    let src = format!(
        "fn mkv<T>(x: T) -> Vec<T> {{\n    let mut v: Vec<T> = Vec::new()\n    Vec::push(v, x)\n\
         \x20   return v\n}}\nfn esc<T>(x: T) -> Vec<&i64> {{\n    let a: [i64; 3] = [7, 8, 41]\n\
         \x20   return mkv(&a[2])\n}}\n{}",
        RC2_75_BURN
    );
    refusal_row("rc2-75", &src, &[], "E0714", "which makes a `Vec<&i64>`");
}

#[test]
fn rc2_76_a_reference_escaping_in_a_vec_made_by_a_generic_from_a_plain_function_is_refused() {
    let src = format!(
        "fn mkv<T>(x: T) -> Vec<T> {{\n    let mut v: Vec<T> = Vec::new()\n    Vec::push(v, x)\n\
         \x20   return v\n}}\nfn esc(x: i64) -> Vec<&i64> {{\n    let a: [i64; 3] = [7, 8, 41]\n\
         \x20   return mkv(&a[2])\n}}\n{}",
        RC2_75_BURN
    );
    refusal_row(
        "rc2-76",
        &src,
        &[],
        "E0714",
        "the call to `mkv` in `esc` instantiates it with `&i64`, which makes a `Vec<&i64>`",
    );
}

#[test]
fn rc2_77_an_rc_in_an_array_made_by_a_dead_local_is_refused_at_every_level() {
    refusal_row(
        "rc2-77",
        "struct N { v: i64 }\nfn two<T>(x: T) -> i64 { let a: [T; 2] = [x, x]; return 2 }\n\
         fn main() -> i64 {\n    let p: Rc<N> = Rc::new(N { v: 9 })\n\
         \x20   println(\"{two(p)} {Rc::get(p).v}\")\n    return 0\n}\n",
        &[],
        "E0410",
        "the call to `two` in `main` instantiates it with `Rc<N>`, which makes a `[Rc<N>; 2]`",
    );
}

const RC2_78_LIB: &str = r#"
pub fn boxit<T>(x: T) -> i64 {
    let r: Rc<T> = Rc::new(x)
    return 1
}



pub fn outer<T>(x: T) -> i64 {
    let k: i64 = 5
    let n: i64 = boxit(x)
    return n + k
}
"#;

#[test]
fn rc2_78_a_refused_instance_from_a_module_points_at_the_call_in_the_root_file() {
    refusal_row(
        "rc2-78",
        "needs lib1\nfn main() -> i64 {\n    let s: [i64; 3] = [7, 0, 41]\n\
         \x20   let n: i64 = lib1.outer(&s[2])\n    println(\"{s[0]} {n}\")\n    return 0\n}\n",
        &[("lib1.aelys", RC2_78_LIB)],
        "E0714",
        "the call to `lib1.outer` in `main` instantiates it with `&i64`, which through \
         `lib1.boxit` makes a `Rc<&i64>`",
    );
    let h = Harness::new();
    let root = h.stage(
        "rc2-78",
        "span",
        "needs lib1\nfn main() -> i64 {\n    let s: [i64; 3] = [7, 0, 41]\n\
         \x20   let n: i64 = lib1.outer(&s[2])\n    println(\"{s[0]} {n}\")\n    return 0\n}\n",
        &[("lib1.aelys", RC2_78_LIB)],
    );
    let text =
        compile_file_with_llvm_variant(&root, OptimizationLevel::None, false, RuntimeVariant::Rc)
            .expect_err("rc2-78 compiled")
            .to_string();
    assert!(text.contains("lib1.outer(&s[2])"), "rc2-78: span\n{text}");
}

#[test]
fn rc2_79_an_rc_read_out_of_an_rc_by_a_generic_is_refused() {
    refusal_row(
        "rc2-79",
        "struct N { v: i64 }\nfn wrap<T>(x: T) -> Rc<T> { return Rc::new(x) }\n\
         fn unwrap<T>(r: Rc<T>) -> T { return Rc::get(r) }\nfn main() -> i64 {\n\
         \x20   let p: Rc<N> = Rc::new(N { v: 9 })\n    let rr: Rc<Rc<N>> = wrap(p)\n\
         \x20   let q: Rc<N> = unwrap(rr)\n    println(\"{Rc::get(q).v} {Rc::get(p).v}\")\n\
         \x20   return 0\n}\n",
        &[],
        "E0410",
        "the call to `unwrap` in `main` instantiates it with `Rc<N>`, which reads an `Rc<N>` out of \
         an `Rc<Rc<N>>`",
    );
}

#[test]
fn rc2_80_an_rc_put_straight_into_a_generic_enum_names_what_is_unsupported() {
    refusal_row(
        "rc2-80",
        "struct N { v: i64 }\nenum One<T> { It(T) }\nfn get1<T>(o: One<T>) -> T {\n\
         \x20   return match o {\n        One::It(v) => v\n    }\n}\nfn main() -> i64 {\n\
         \x20   let q: Rc<N> = get1(One::It(Rc::new(N { v: 4 })))\n    println(\"{Rc::get(q).v}\")\n\
         \x20   return 0\n}\n",
        &[],
        "E0410",
        "carrying an `Rc<T>` straight into a generic enum, which is not supported yet",
    );
}

const RC2_81: &str = r#"
struct N { s: string }
struct W { r: Rc<N> }
struct H { w: W }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
fn f(w: W, k: i64) -> i64 {
    let b: i64 = Rc::get(w.r).s.bytes[0]
    return b * 1000 + Rc::get(w.r).s.len + k
}
fn main() -> i64 {
    let mut h = H { w: W { r: Rc::new(N { s: victim(12345) }) } }
    let v = f(h.w, { h = H { w: W { r: Rc::new(N { s: other(9) }) } }
        let y = Rc::new(N { s: other(88888) })
        Rc::get(y).s.len * 0 })
    println("{v}")
    return 0
}
"#;

#[test]
fn rc2_81_a_carrier_field_argument_survives_a_sibling_reassigning_its_owner() {
    let h = Harness::new();
    h.differential_row("rc2-81", RC2_81, &[], "97007\n", (9, 9));
    h.lsan_row("rc2-81", RC2_81, &[], "97007\n");
    h.assert_legs(24);
}

const RC2_82: &str = r#"
struct N { s: string }
struct H { r: Rc<N> }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
fn f(r: Rc<N>, k: i64) -> i64 {
    let b: i64 = Rc::get(r).s.bytes[0]
    return b * 1000 + Rc::get(r).s.len + k
}
fn main() -> i64 {
    let mut h = H { r: Rc::new(N { s: victim(12345) }) }
    let v = f(h.r, { h = H { r: Rc::new(N { s: other(9) }) }
        let y = Rc::new(N { s: other(88888) })
        Rc::get(y).s.len * 0 })
    println("{v}")
    return 0
}
"#;

#[test]
fn rc2_82_an_rc_field_argument_survives_a_sibling_reassigning_its_owner() {
    let h = Harness::new();
    h.differential_row("rc2-82", RC2_82, &[], "97007\n", (9, 9));
    h.lsan_row("rc2-82", RC2_82, &[], "97007\n");
    h.assert_legs(24);
}

const RC2_83: &str = r#"
struct N { k: i64 }
struct W { r: Rc<N> }
fn f(w: W, k: i64) -> i64 { return Rc::get(w.r).k * 100 + k }
fn main() -> i64 {
    let mut w = W { r: Rc::new(N { k: 1 }) }
    let v = f(w, { w = W { r: Rc::new(N { k: 5 }) }
        0 })
    println("{v} {Rc::get(w.r).k}")
    return 0
}
"#;

#[test]
fn rc2_83_a_carrier_argument_keeps_its_value_when_a_sibling_reassigns_the_local() {
    let h = Harness::new();
    h.differential_row("rc2-83", RC2_83, &[], "100 5\n", (6, 6));
    h.lsan_row("rc2-83", RC2_83, &[], "100 5\n");
    h.assert_legs(24);
}

const RC2_84: &str = r#"
struct N { k: i64 }
struct W { r: Rc<N>, j: i64 }
fn f(w: W, k: i64) -> i64 { return Rc::get(w.r).k * 100 + w.j * 10 + k }
fn main() -> i64 {
    let mut w = W { r: Rc::new(N { k: 1 }), j: 1 }
    let v = f(w, { w.j = 7
        0 })
    println("{v} {w.j}")
    return 0
}
"#;

#[test]
fn rc2_84_a_carrier_argument_keeps_its_value_when_a_sibling_writes_one_of_its_fields() {
    let h = Harness::new();
    h.differential_row("rc2-84", RC2_84, &[], "110 7\n", (5, 5));
    h.lsan_row("rc2-84", RC2_84, &[], "110 7\n");
    h.assert_legs(24);
}

const RC2_85: &str = r#"
struct N { k: i64 }
struct M { s: string, r: Rc<N> }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
fn f(a: M, k: i64) -> i64 {
    let b: i64 = a.s.bytes[0]
    return b * 1000 + Rc::get(a.r).k * 10 + k
}
fn main() -> i64 {
    let mut x = M { s: victim(12345), r: Rc::new(N { k: 1 }) }
    let v = f(x, { x = M { s: other(9), r: Rc::new(N { k: 5 }) }
        let y = other(88888)
        y.len * 0 })
    println("{v}")
    return 0
}
"#;

#[test]
fn rc2_85_a_string_and_rc_argument_takes_one_share_when_a_sibling_reassigns_the_local() {
    let h = Harness::new();
    h.differential_row("rc2-85", RC2_85, &[], "97010\n", (8, 8));
    h.lsan_row("rc2-85", RC2_85, &[], "97010\n");
    h.assert_legs(24);
}

const MULTI_VARIANT_RC: &str = "an enum with more than one variant cannot carry an Rc yet";

#[test]
fn rc2_86_a_linked_list_enum_carrying_an_rc_is_refused_at_its_declaration() {
    refusal_row_full(
        "rc2-86",
        "struct Node { val: i64, next: Link }\nenum Link { Nil, Cons(Rc<Node>) }\n\
         fn sum(l: Link) -> i64 {\n    match l {\n\
         \x20       Link::Cons(n) => { return Rc::get(n).val + sum(Rc::get(n).next) }\n\
         \x20       Link::Nil => { return 0 }\n    }\n}\nfn main() -> i64 {\n\
         \x20   let c = Rc::new(Node { val: 3, next: Link::Nil })\n\
         \x20   let b = Rc::new(Node { val: 2, next: Link::Cons(c) })\n\
         \x20   println(\"sum={sum(Link::Cons(b))}\")\n    return 0\n}\n",
        &[],
        "E0410",
        &[
            "variant `Link::Cons` has a payload of type `Rc<Node>`, which holds an `Rc`",
            MULTI_VARIANT_RC,
            "enum Link { Nil, Cons(Rc<Node>) }",
        ],
        &["is given a value of type"],
    );
}

#[test]
fn rc2_87_a_linked_list_built_through_a_generic_identity_is_refused_at_its_declaration() {
    refusal_row_full(
        "rc2-87",
        "struct Node { val: i64, next: Link }\nenum Link { Nil, Cons(Rc<Node>) }\n\
         fn id<T>(x: T) -> T { return x }\nfn sum(l: Link) -> i64 {\n    match l {\n\
         \x20       Link::Cons(n) => { return Rc::get(n).val + sum(Rc::get(n).next) }\n\
         \x20       Link::Nil => { return 0 }\n    }\n}\nfn once() -> i64 {\n\
         \x20   let c = id(Rc::new(Node { val: 3, next: Link::Nil }))\n\
         \x20   let b = id(Rc::new(Node { val: 2, next: Link::Cons(c) }))\n\
         \x20   return sum(Link::Cons(b))\n}\nfn main() -> i64 {\n    let mut t: i64 = 0\n\
         \x20   let mut i: i64 = 0\n    while i < 1000 { t = t + once()\n        i = i + 1 }\n\
         \x20   println(\"{t}\")\n    return 0\n}\n",
        &[],
        "E0410",
        &[
            "variant `Link::Cons` has a payload of type `Rc<Node>`",
            MULTI_VARIANT_RC,
        ],
        &[],
    );
}

#[test]
fn rc2_88_an_rc_handed_to_a_multi_variant_enum_through_a_generic_binding_is_refused() {
    refusal_row_full(
        "rc2-88",
        "struct N { v: i64 }\nenum L { Num(i64), R(Rc<N>) }\nfn id<T>(x: T) -> T { return x }\n\
         fn val(l: L) -> i64 { match l { L::Num(x) => { return x } L::R(n) => { return Rc::get(n).v } } }\n\
         fn main() -> i64 {\n    let mut t: i64 = 0\n    let mut i: i64 = 0\n\
         \x20   while i < 1000 { let r = Rc::new(N { v: 1 })\n        let r2 = id(r)\n\
         \x20       t = t + val(L::R(r2))\n        i = i + 1 }\n    println(\"{t}\")\n\
         \x20   return 0\n}\n",
        &[],
        "E0410",
        &[
            "variant `L::R` has a payload of type `Rc<N>`",
            MULTI_VARIANT_RC,
        ],
        &[],
    );
}

#[test]
fn rc2_89_an_rc_carrier_handed_to_a_multi_variant_enum_through_a_generic_binding_is_refused() {
    refusal_row_full(
        "rc2-89",
        "struct N { v: i64 }\nstruct W { r: Rc<N> }\nenum L { Num(i64), R(W) }\n\
         fn id<T>(x: T) -> T { return x }\n\
         fn val(l: L) -> i64 { match l { L::Num(x) => { return x } L::R(w) => { return Rc::get(w.r).v } } }\n\
         fn main() -> i64 {\n    let mut t: i64 = 0\n    let mut i: i64 = 0\n\
         \x20   while i < 1000 { let w = W { r: Rc::new(N { v: 1 }) }\n        let w2 = id(w)\n\
         \x20       t = t + val(L::R(w2))\n        i = i + 1 }\n    println(\"{t}\")\n\
         \x20   return 0\n}\n",
        &[],
        "E0410",
        &["variant `L::R` has a payload of type `W`", MULTI_VARIANT_RC],
        &[],
    );
}

#[test]
fn rc2_90_a_multi_variant_enum_carrying_an_imported_rc_carrier_is_refused() {
    refusal_row_full(
        "rc2-90",
        "needs m\nenum L { Nil, V(m.W) }\nfn main() -> i64 { println(\"ok\"); return 0 }\n",
        &[("m.aelys", "pub struct W { pub r: Rc<i64> }\n")],
        "E0410",
        &[
            "variant `L::V` has a payload of type `m.W`",
            MULTI_VARIANT_RC,
        ],
        &[],
    );
}

#[test]
fn rc2_91_a_generic_multi_variant_enum_with_a_concrete_rc_payload_is_refused() {
    refusal_row_full(
        "rc2-91",
        "struct N { v: i64 }\nenum L<T> { Nil, V(T), R(Rc<N>) }\n\
         fn main() -> i64 { println(\"ok\"); return 0 }\n",
        &[],
        "E0410",
        &[
            "variant `L::R` has a payload of type `Rc<N>`",
            MULTI_VARIANT_RC,
        ],
        &[],
    );
}

#[test]
fn rc2_92_an_rc_through_a_single_variant_enum_of_a_carrier_is_refused_at_the_outer_enum() {
    refusal_row_full(
        "rc2-92",
        "struct W { k: i64, r: Rc<i64> }\nenum One { It(W) }\nenum L { Nil, V(One) }\n\
         fn main() -> i64 { println(\"ok\"); return 0 }\n",
        &[],
        "E0410",
        &[
            "variant `L::V` has a payload of type `One`",
            MULTI_VARIANT_RC,
        ],
        &[],
    );
}

const GENERIC_STRUCT: &str = "generic structs are not supported yet";

#[test]
fn rc2_93_a_generic_struct_declared_and_unused_is_refused() {
    refusal_row_full(
        "rc2-93",
        "struct Box<T> { v: T }\nfn main() -> i64 { println(\"ok\"); return 0 }\n",
        &[],
        "E0902",
        &["struct `Box<T>` is generic", GENERIC_STRUCT],
        &[],
    );
}

#[test]
fn rc2_94_a_generic_struct_literal_is_refused_instead_of_an_internal_error() {
    refusal_row_full(
        "rc2-94",
        "struct Box<T> { v: T }\nfn main() -> i64 { let b: Box<i64> = Box { v: 5 }\n\
         \x20println(\"ok\"); return 0 }\n",
        &[],
        "E0902",
        &[GENERIC_STRUCT],
        &["E0901"],
    );
}

#[test]
fn rc2_95_a_generic_struct_field_read_is_refused_at_the_declaration() {
    refusal_row_full(
        "rc2-95",
        "struct Box<T> { v: T }\nfn main() -> i64 { let b: Box<i64> = Box { v: 5 }\n\
         \x20println(\"{b.v}\"); return 0 }\n",
        &[],
        "E0902",
        &[GENERIC_STRUCT, "struct Box<T> { v: T }"],
        &["E0304"],
    );
}

#[test]
fn rc2_96_a_generic_struct_with_an_inferred_literal_is_refused_at_the_declaration() {
    refusal_row_full(
        "rc2-96",
        "struct Pair<A, B> { a: A, b: B }\nfn main() -> i64 { let p = Pair { a: 5, b: 6 }\n\
         \x20println(\"{p.a + 1}\"); return 0 }\n",
        &[],
        "E0902",
        &["struct `Pair<A, B>` is generic", GENERIC_STRUCT],
        &["E0301"],
    );
}

#[test]
fn rc2_97_a_global_initialized_by_a_call_is_unsupported_at_its_let() {
    refusal_row_full(
        "rc2-97",
        "fn mk() -> i64 { return 7 }\n\nlet G = mk()\nfn main() -> i64 { println(\"{G}\"); return 0 }\n",
        &[],
        "E0902",
        &[
            "[E0902]: file-scope let 'G' requires a compile-time constant initializer",
            "let G = mk()",
        ],
        &["E0904", "[air-lowering]"],
    );
}

#[test]
fn rc2_98_a_lowering_refusal_names_its_site_without_a_numbered_prefix() {
    refusal_row_full(
        "rc2-98",
        "fn size() -> i64 { return 3 }\nfn main() -> i64 {\n    let n: i64 = size()\n\
         \x20   let a = [0; n]\n    println(\"{a[0]}\")\n    return 0\n}\n",
        &[],
        "E0902",
        &[
            "[E0902]: unsupported non-constant array size",
            "let a = [0; n]",
        ],
        &["[air-lowering]", "fn size() -> i64 { return 3 }"],
    );
}

#[test]
fn rc2_99_a_slice_of_a_string_names_substring() {
    refusal_row_full(
        "rc2-99",
        "struct Name { first: string, last: string }\nfn parse(full: string) -> Name {\n\
         \x20   let mut i: i64 = 0\n    while i < full.len && full.bytes[i] != 32 as u8 { i = i + 1 }\n\
         \x20   return Name { first: full[0..i], last: full[i + 1..full.len] }\n}\n\
         fn main() -> i64 {\n    let n = parse(\"ada lovelace\")\n\
         \x20   println(\"{n.last}, {n.first}\")\n    return 0\n}\n",
        &[],
        "E0425",
        &["a `string` has no slices", "str.substring"],
        &["&[dynamic]"],
    );
}

#[test]
fn rc2_100_a_vec_field_refusal_does_not_mention_rc() {
    refusal_row_full(
        "rc2-100",
        "struct Bag { items: Vec<i64> }\nfn main() -> i64 {\n    let b = Bag { items: Vec::new() }\n\
         \x20   return 0\n}\n",
        &[],
        "E0902",
        &[
            "field `items` of struct `Bag` has type `Vec<i64>`: a Vec inside a struct is not \
             supported in 0.1; keep the Vec in its own binding and pass it with `&mut`, or use a \
             fixed-size array field",
        ],
        &["Rc used outside", "[rc-stage1]", "Rc"],
    );
}

#[test]
fn rc2_101_a_tuple_type_says_tuples_are_unsupported() {
    refusal_row_full(
        "rc2-101",
        "fn divmod(a: i64, b: i64) -> (i64, i64) { return (a / b, a % b) }\n\
         fn main() -> i64 { return 0 }\n",
        &[],
        "E0902",
        &["tuples are not supported yet"],
        &["expected type name"],
    );
}

#[test]
fn rc2_102_a_tuple_expression_says_tuples_are_unsupported() {
    refusal_row_full(
        "rc2-102",
        "fn main() -> i64 {\n    let t = (1, 2)\n    return 0\n}\n",
        &[],
        "E0902",
        &["tuples are not supported yet"],
        &["expected ), found ,"],
    );
}

const LATE_HELP: &str = "no implicit conversion applies to a value that crosses a binding whose \
                         type is only inferred later";

const RC2_103: &str = r#"
struct In { b: i64 }
struct Out { a: In }
fn id<T>(x: T) -> T { return x }
fn main() -> i64 {
    let o = id(Out { a: In { b: 7 } })
    println("{o.a.b}")
    return 0
}
"#;

#[test]
fn rc2_103_a_field_chain_through_a_generic_result_reads_the_inner_field() {
    let h = Harness::new();
    h.differential_row("rc2-103", RC2_103, &[], "7\n", (0, 0));
    h.assert_legs(18);
}

const RC2_104: &str = r#"
fn id<T>(x: T) -> T { return x }
fn main() -> i64 {
    let s = id("abc")
    if s.len > 2 { println("{s.len}") } else { println("small") }
    return 0
}
"#;

#[test]
fn rc2_104_the_length_of_a_string_returned_by_a_generic_is_its_byte_count() {
    let h = Harness::new();
    h.differential_row("rc2-104", RC2_104, &[], "3\n", (0, 0));
    h.assert_legs(18);
}

const RC2_105: &str = r#"
fn id<T>(x: T) -> T { return x }
fn main() -> i64 {
    let v: i64 = 41
    let p = id(&v)
    let y: i64 = *p + 1
    println("{y}")
    return 0
}
"#;

#[test]
fn rc2_105_a_reference_returned_by_a_generic_dereferences_to_its_referent() {
    let h = Harness::new();
    h.differential_row("rc2-105", RC2_105, &[], "42\n", (0, 0));
    h.assert_legs(18);
}

const RC2_106: &str = r#"
struct P { a: i32, b: i32 }
fn id<T>(x: T) -> T { return x }
fn main() -> i64 {
    let mut p = id(P { a: 3, b: 4 })
    let v: i32 = -9
    p.a = v
    println("{p.a} {p.b}")
    return 0
}
"#;

#[test]
fn rc2_106_a_field_write_into_a_generic_result_keeps_its_neighbour() {
    let h = Harness::new();
    h.differential_row("rc2-106", RC2_106, &[], "-9 4\n", (4, 4));
    h.assert_legs(18);
}

const RC2_107: &str = r#"
fn id<T>(x: T) -> T { return x }
fn main() -> i64 {
    let b: [i32; 2] = [1, 2]
    let mut a = id(b)
    let v: i32 = -7
    a[0] = v
    println("{a[0]} {a[1]}")
    return 0
}
"#;

#[test]
fn rc2_107_an_element_write_into_a_generic_result_keeps_its_neighbour() {
    let h = Harness::new();
    h.differential_row("rc2-107", RC2_107, &[], "-7 2\n", (4, 4));
    h.assert_legs(18);
}

const RC2_108: &str = r#"
struct P { a: i64, b: i64 }
fn id<T>(x: T) -> T { return x }
fn main() -> i64 {
    let mut p = id(P { a: 3, b: 4 })
    p.a += 10
    let mut a = id([1, 2])
    a[1] *= 21
    println("{p.a} {p.b} {a[1]}")
    return 0
}
"#;

#[test]
fn rc2_108_a_compound_assignment_into_a_generic_result_reads_and_writes_the_same_slot() {
    let h = Harness::new();
    h.differential_row("rc2-108", RC2_108, &[], "13 4 42\n", (7, 7));
    h.assert_legs(18);
}

const RC2_109: &str = r#"
struct P { a: i32, b: i32 }
enum May<T> { Yes(T), No }
fn main() -> i64 {
    let m = May::Yes(P { a: -1, b: 1 })
    match m {
        May::Yes(p) => { println("{p.a}") }
        May::No => { println("no") }
    }
    return 0
}
"#;

#[test]
fn rc2_109_a_field_of_a_generic_enum_payload_keeps_its_sign() {
    let h = Harness::new();
    h.differential_row("rc2-109", RC2_109, &[], "-1\n", (0, 0));
    h.assert_legs(18);
}

const RC2_110: &str = r#"
struct O { arr: [i64; 3] }
fn id<T>(x: T) -> T { return x }
fn main() -> i64 {
    let o = id(O { arr: [1, 2, 3] })
    println("{o.arr[1]}")
    return 0
}
"#;

#[test]
fn rc2_110_an_element_of_a_field_of_a_generic_result_is_read_at_its_type() {
    let h = Harness::new();
    h.differential_row("rc2-110", RC2_110, &[], "2\n", (0, 0));
    h.assert_legs(18);
}

#[test]
fn rc2_111_a_narrow_field_stored_into_a_wide_local_after_a_generic_is_refused() {
    refusal_row_full(
        "rc2-111",
        "struct P { a: i32, b: i32 }\nfn id<T>(x: T) -> T { return x }\n\
         fn main() -> i64 {\n    let p = id(P { a: -1, b: 100 })\n    let mut y: i64 = 7777777777\n\
         \x20   y = p.a\n    println(\"{y}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &["expected `i64`, found `i32`", LATE_HELP],
        &["E0901"],
    );
}

#[test]
fn rc2_112_a_narrow_field_bound_to_a_wide_annotation_after_a_generic_is_refused() {
    refusal_row_full(
        "rc2-112",
        "struct P { a: i32, b: i32 }\nfn id<T>(x: T) -> T { return x }\n\
         fn main() -> i64 {\n    let p = id(P { a: -1, b: 100 })\n    let y: i64 = p.a\n\
         \x20   println(\"{y}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &["expected `i64`, found `i32`", LATE_HELP],
        &["E0901"],
    );
}

#[test]
fn rc2_113_a_narrow_field_of_a_generic_enum_payload_stored_wide_is_refused() {
    refusal_row_full(
        "rc2-113",
        "struct P { a: i32, b: i32 }\nenum May<T> { Yes(T), No }\n\
         fn main() -> i64 {\n    let m = May::Yes(P { a: -1, b: 1 })\n    let mut y: i64 = 0\n\
         \x20   match m { May::Yes(p) => { y = p.a } May::No => { y = 0 } }\n\
         \x20   println(\"{y}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &["expected `i64`, found `i32`", LATE_HELP],
        &["E0901"],
    );
}

#[test]
fn rc2_114_an_integer_field_bound_as_a_string_after_a_generic_is_refused() {
    refusal_row_full(
        "rc2-114",
        "struct P { a: i64 }\nfn id<T>(x: T) -> T { return x }\n\
         fn main() -> i64 {\n    let p = id(P { a: 123456 })\n    let s: string = p.a\n\
         \x20   println(\"{s.len}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &["expected `string`, found `i64`"],
        &["E0901", LATE_HELP],
    );
}

#[test]
fn rc2_115_an_integer_element_bound_as_a_string_after_a_generic_is_refused() {
    refusal_row_full(
        "rc2-115",
        "fn id<T>(x: T) -> T { return x }\nfn main() -> i64 {\n\
         \x20   let a: [i64; 2] = [123456, 7]\n    let b = id(a)\n    let s: string = b[0]\n\
         \x20   println(\"{s.len}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &["expected `string`, found `i64`"],
        &["E0901"],
    );
}

#[test]
fn rc2_116_an_integer_loop_element_bound_as_a_string_after_a_generic_is_refused() {
    refusal_row_full(
        "rc2-116",
        "fn id<T>(x: T) -> T { return x }\nfn main() -> i64 {\n\
         \x20   let a: [i64; 2] = [123456, 7]\n    let b = id(a)\n    let mut t: i64 = 0\n\
         \x20   for s in b {\n        let q: string = s\n        t = t + q.len\n    }\n\
         \x20   println(\"{t}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &["expected `string`, found `i64`"],
        &["E0901"],
    );
}

#[test]
fn rc2_117_a_narrow_field_of_an_unannotated_lambda_parameter_stored_wide_is_refused() {
    refusal_row_full(
        "rc2-117",
        "struct P { a: i32, b: i32 }\nfn main() -> i64 {\n\
         \x20   let f = fn(p) -> i64 {\n        let y: i64 = p.a\n        return y\n    }\n\
         \x20   println(\"{f(P { a: -1, b: 1 })}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &[
            "expected `i64`, found `i32`",
            "field `a` of a value whose type is only inferred after this use",
            LATE_HELP,
        ],
        &["E0901"],
    );
}

#[test]
fn rc2_118_a_string_stored_into_an_integer_field_of_a_generic_result_is_refused() {
    refusal_row_full(
        "rc2-118",
        "struct P { a: i64, b: i64 }\nfn id<T>(x: T) -> T { return x }\n\
         fn get(p: P) -> i64 { return p.a }\nfn main() -> i64 {\n\
         \x20   let mut p = id(P { a: 3, b: 4 })\n    p.a = \"hello\"\n\
         \x20   println(\"{get(p)}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &[
            "expected `i64`, found `string`",
            "assignment to field `a` of a value whose type is only inferred after this use",
        ],
        &["E0901"],
    );
}

#[test]
fn rc2_119_a_wide_value_stored_into_a_narrow_field_of_a_generic_result_is_refused() {
    refusal_row_full(
        "rc2-119",
        "struct P { a: i32, b: i32 }\nfn id<T>(x: T) -> T { return x }\n\
         fn get(p: P) -> i32 { return p.b }\nfn main() -> i64 {\n\
         \x20   let mut p = id(P { a: 3, b: 4 })\n    let big: i64 = 77777777777\n    p.a = big\n\
         \x20   println(\"{get(p)}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &["expected `i32`, found `i64`", LATE_HELP],
        &["E0901"],
    );
}

#[test]
fn rc2_120_a_string_stored_into_an_integer_element_of_a_generic_result_is_refused() {
    refusal_row_full(
        "rc2-120",
        "fn id<T>(x: T) -> T { return x }\nfn get(a: [i64; 2]) -> i64 { return a[0] + a[1] }\n\
         fn main() -> i64 {\n    let mut a = id([1, 2])\n    a[0] = \"hello\"\n\
         \x20   println(\"{get(a)}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &["expected `i64`, found `string`", "element assignment"],
        &["E0901"],
    );
}

#[test]
fn rc2_121_a_wide_value_stored_into_a_narrow_element_of_a_generic_result_is_refused() {
    refusal_row_full(
        "rc2-121",
        "fn id<T>(x: T) -> T { return x }\nfn get(a: [i32; 2]) -> i32 { return a[1] }\n\
         fn main() -> i64 {\n    let b: [i32; 2] = [1, 2]\n    let mut a = id(b)\n\
         \x20   let big: i64 = 77777777777\n    a[0] = big\n    println(\"{get(a)}\")\n\
         \x20   return 0\n}\n",
        &[],
        "E0301",
        &["expected `i32`, found `i64`", LATE_HELP],
        &["E0901"],
    );
}

const RC2_122: &str = r#"
struct N { s: string }
struct H { r: Rc<N>, k: i64 }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn id<T>(x: T) -> T { return x }
fn main() -> i64 {
    let mut i: i64 = 0
    let mut t: i64 = 0
    while i < 1000 {
        let mut h = id(H { r: Rc::new(N { s: victim(i) }), k: 1 })
        let q = Rc::new(N { s: victim(i + 7) })
        h.r = q
        let w: H = h
        t = t + w.k + Rc::get(w.r).s.len
        i = i + 1
    }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_122_an_rc_field_write_into_a_generic_result_releases_the_old_value() {
    let h = Harness::new();
    h.differential_row("rc2-122", RC2_122, &[], "5911\n", (6000, 6000));
    h.lsan_row("rc2-122", RC2_122, &[], "5911\n");
    h.assert_legs(24);
}

#[test]
fn rc2_123_a_wide_value_written_through_a_reference_from_a_generic_is_refused() {
    refusal_row_full(
        "rc2-123",
        "fn id<T>(x: T) -> T { return x }\nfn main() -> i64 {\n    let mut x: i32 = 5\n\
         \x20   let p = id(&mut x)\n    let big: i64 = 77777777777\n    *p = big\n\
         \x20   println(\"{x}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &[
            "expected `i32`, found `i64`",
            "assignment through a reference of a value whose type is only inferred after this use",
        ],
        &["E0901"],
    );
}

#[test]
fn rc2_124_a_narrow_slice_element_stored_wide_after_a_generic_is_refused() {
    refusal_row_full(
        "rc2-124",
        "fn id<T>(x: T) -> T { return x }\nfn main() -> i64 {\n\
         \x20   let a: [i32; 3] = [-1, 2, 3]\n    let b = id(a)\n    let s = b[0..2]\n\
         \x20   let y: i64 = s[0]\n    println(\"{y}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &["expected `i64`, found `i32`"],
        &["E0901"],
    );
}

#[test]
fn rc2_125_a_field_of_a_value_whose_type_is_never_inferred_asks_for_an_annotation() {
    refusal_row_full(
        "rc2-125",
        "fn main() -> i64 {\n    let f = fn(p) -> i64 { return p.a }\n    println(\"ok\")\n\
         \x20   return 0\n}\n",
        &[],
        "E0301",
        &[
            "the type of this value is never inferred, so its field `a` cannot be resolved; \
             annotate the binding it comes from",
            "return p.a",
        ],
        &["E0412", "survived lowering", "E0304"],
    );
}

#[test]
fn rc2_126_a_private_field_write_into_a_generic_result_is_refused() {
    refusal_row_full(
        "rc2-126",
        "needs m\nfn id<T>(x: T) -> T { return x }\nfn main() -> i64 {\n\
         \x20   let mut p = id(m.mk())\n    p.b = 5\n    println(\"{m.getb(p)}\")\n    return 0\n}\n",
        &[(
            "m.aelys",
            "pub struct P { pub a: i64, b: i64 }\npub fn mk() -> P { return P { a: 1, b: 2 } }\n\
             pub fn getb(p: P) -> i64 { return p.b }\n",
        )],
        "E0610",
        &["field `b` of `m.P` is not public"],
        &["E0901"],
    );
}

#[test]
fn rc2_127_rc_get_of_a_value_typed_only_later_asks_for_an_annotation() {
    refusal_row_full(
        "rc2-127",
        "struct N { k: i64 }\nfn id<T>(x: T) -> T { return x }\nfn main() -> i64 {\n\
         \x20   let r = Rc::new(N { k: 4 })\n    let q = id(r)\n    println(\"{Rc::get(q).k}\")\n\
         \x20   return 0\n}\n",
        &[],
        "E0301",
        &[
            "the argument of `Rc::get` has a type that is only inferred after this use; annotate \
             the binding it comes from as `Rc<T>`",
        ],
        &["E0901", "τ"],
    );
}

#[test]
fn rc2_128_a_constant_index_past_an_array_returned_by_a_generic_is_refused() {
    refusal_row_full(
        "rc2-128",
        "fn id<T>(x: T) -> T { return x }\nfn main() -> i64 {\n    let a = id([1, 2, 3])\n\
         \x20   println(\"{a[5]}\")\n    return 0\n}\n",
        &[],
        "E0433",
        &["index 5 is outside `[_; 3]`"],
        &["E0412", "E0901"],
    );
}

#[test]
fn rc2_129_an_integer_index_into_a_string_returned_by_a_generic_is_refused() {
    refusal_row_full(
        "rc2-129",
        "fn id<T>(x: T) -> T { return x }\nfn main() -> i64 {\n    let s = id(\"abc\")\n\
         \x20   println(\"{s[0]}\")\n    return 0\n}\n",
        &[],
        "E0304",
        &["a `string` cannot be indexed by integer"],
        &["E0412", "E0901"],
    );
}

#[test]
fn rc2_130_a_compound_assignment_of_a_wide_value_into_a_narrow_generic_field_is_refused() {
    refusal_row_full(
        "rc2-130",
        "struct P { a: i32, b: i32 }\nfn id<T>(x: T) -> T { return x }\n\
         fn get(p: P) -> i32 { return p.b }\nfn main() -> i64 {\n\
         \x20   let mut p = id(P { a: 3, b: 4 })\n    let big: i64 = 77777777777\n    p.a += big\n\
         \x20   println(\"{get(p)}\")\n    return 0\n}\n",
        &[],
        "E0301",
        &["expected `i32`, found `i64`"],
        &["E0901"],
    );
}

const RC2_131: &str = r#"
struct P { a: i32, b: i32 }
fn main() -> i64 {
    let mut p = P { a: -1, b: 100 }
    let y: i64 = p.a
    p.a = 5
    let z = p.a * 2
    let mut arr: [i32; 2] = [1, 2]
    arr[0] = 3
    let w: i64 = arr[0]
    println("{y} {z} {p.b} {w}")
    return 0
}
"#;

#[test]
fn rc2_131_concrete_field_and_element_accesses_still_widen_and_narrow() {
    let h = Harness::new();
    h.differential_row("rc2-131", RC2_131, &[], "-1 10 100 3\n", (10, 10));
    h.assert_legs(18);
}

const RC2_132: &str = r#"
struct N { s: string }
struct H { r: Rc<N>, k: i64 }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut h = H { r: Rc::new(N { s: victim(1) }), k: 1 }
    let q = Rc::new(N { s: victim(8) })
    h.r = q
    println("{Rc::get(h.r).s} {Rc::get(q).s}")
    return 0
}
"#;

#[test]
fn rc2_132_an_rc_field_write_into_a_concrete_local_releases_the_old_value() {
    let h = Harness::new();
    h.differential_row("rc2-132", RC2_132, &[], "ab8 ab8\n", (8, 8));
    h.lsan_row("rc2-132", RC2_132, &[], "ab8 ab8\n");
    h.assert_legs(24);
}

const SHARED_RESLICE: &str = "[shared-mut] a mutable slice writes through a shared";

#[test]
fn rc2_133_a_mutable_reslice_of_string_bytes_from_a_generic_is_refused() {
    refusal_row_full(
        "rc2-133",
        "fn id<T>(x: T) -> T { return x }\nfn main() -> i64 {\n    let s: string = \"hello\"\n\
         \x20   let mut t = id(s.bytes)\n    let u = t[0..2]\n    let h: u8 = 72\n    u[0] = h\n\
         \x20   println(\"{s}\")\n    return 0\n}\n",
        &[],
        "E0422",
        &[SHARED_RESLICE, "`&[u8]`"],
        &["E0901"],
    );
}

#[test]
fn rc2_134_a_mutable_reslice_of_heap_string_bytes_from_a_generic_is_refused() {
    refusal_row_full(
        "rc2-134",
        "fn id<T>(x: T) -> T { return x }\nfn mk(n: i64) -> string { return \"ab\" + \"{n}\" }\n\
         fn main() -> i64 {\n    let s: string = mk(5)\n    let k = s\n    let mut t = id(s.bytes)\n\
         \x20   let u = t[0..2]\n    let h: u8 = 90\n    u[0] = h\n    println(\"{k} {s}\")\n\
         \x20   return 0\n}\n",
        &[],
        "E0422",
        &[SHARED_RESLICE, "`&[u8]`"],
        &["E0901"],
    );
}

#[test]
fn rc2_135_a_mutable_reslice_of_a_shared_slice_from_a_generic_is_refused() {
    refusal_row_full(
        "rc2-135",
        "fn id<T>(x: T) -> T { return x }\nfn main() -> i64 {\n    let a: [i64; 3] = [1, 2, 3]\n\
         \x20   let s = a[0..3]\n    let mut t = id(s)\n    let u = t[0..2]\n    u[0] = 9\n\
         \x20   println(\"{a[0]}\")\n    return 0\n}\n",
        &[],
        "E0422",
        &[SHARED_RESLICE, "`&[i64]`"],
        &["E0901"],
    );
}

#[test]
fn rc2_136_a_shared_slice_from_a_generic_resliced_into_a_mutable_parameter_is_refused() {
    refusal_row_full(
        "rc2-136",
        "fn id<T>(x: T) -> T { return x }\nfn poke(s: &mut [i64]) -> i64 { s[0] = 9\n return 0 }\n\
         fn main() -> i64 {\n    let a: [i64; 3] = [1, 2, 3]\n    let mut t = id(a[0..3])\n\
         \x20   poke(t[0..3])\n    println(\"{a[0]}\")\n    return 0\n}\n",
        &[],
        "E0422",
        &[SHARED_RESLICE, "`&[i64]`"],
        &["E0901"],
    );
}

#[test]
fn rc2_137_a_mutable_reslice_of_a_shared_slice_through_a_lambda_parameter_is_refused() {
    refusal_row_full(
        "rc2-137",
        "fn main() -> i64 {\n    let a: [i64; 3] = [1, 2, 3]\n    let f = fn(mut t) -> i64 {\n\
         \x20       let u = t[0..2]\n        u[0] = 9\n        return 0\n    }\n\
         \x20   let s = a[0..3]\n    f(s)\n    println(\"{a[0]}\")\n    return 0\n}\n",
        &[],
        "E0422",
        &[SHARED_RESLICE, "`&[i64]`"],
        &["E0901"],
    );
}

const RC2_138: &str = r#"
fn id<T>(x: T) -> T { return x }
fn poke(s: &mut [i64]) -> i64 { s[0] = 9
 return 0 }
fn main() -> i64 {
    let mut a: [i64; 3] = [1, 2, 3]
    let mut t = id(a[0..3])
    poke(t[0..3])
    println("{a[0]}")
    return 0
}
"#;

#[test]
fn rc2_138_a_mutable_slice_from_a_generic_still_reslices_into_a_mutable_parameter() {
    let h = Harness::new();
    h.differential_row("rc2-138", RC2_138, &[], "9\n", (0, 0));
    h.assert_legs(18);
}

const RC2_LATE_CARRIER: &str = "struct N { s: string }\nstruct H { r: Rc<N>, k: i64 }\n\
     fn victim(n: i64) -> string { return \"ab\" + \"{n}\" }\nfn id<T>(x: T) -> T { return x }\n\
     enum May<T> { Yes(T), No }\n";

const GENERIC_ENUM_RC: &str = "payload 0 of enum variant `May::Yes` stores a value of type \
                               `Rc<N>` carrying an `Rc<T>` straight into a generic enum";

#[test]
fn rc2_139_an_rc_field_of_a_generic_result_put_in_a_generic_enum_is_refused() {
    let src = format!(
        "{RC2_LATE_CARRIER}fn main() -> i64 {{\n    let mut i: i64 = 0\n    let mut t: i64 = 0\n\
         \x20   while i < 1000 {{\n        let h = id(H {{ r: Rc::new(N {{ s: victim(i) }}), k: 1 }})\n\
         \x20       let m = May::Yes(h.r)\n        let junk = victim(3)\n\
         \x20       match m {{ May::Yes(x) => {{ t = t + 1 }} May::No => {{ t = t + 0 }} }}\n\
         \x20       i = i + 1\n    }}\n    println(\"{{t}}\")\n    return 0\n}}\n"
    );
    refusal_row_full(
        "rc2-139",
        &src,
        &[],
        "E0410",
        &[GENERIC_ENUM_RC],
        &["E0901"],
    );
}

#[test]
fn rc2_140_an_rc_returned_by_a_generic_put_in_a_generic_enum_is_refused() {
    let src = format!(
        "{RC2_LATE_CARRIER}fn main() -> i64 {{\n    let mut i: i64 = 0\n    let mut t: i64 = 0\n\
         \x20   while i < 1000 {{\n        let r = Rc::new(N {{ s: victim(i) }})\n        let q = id(r)\n\
         \x20       let m = May::Yes(q)\n        let junk = victim(3)\n\
         \x20       match m {{ May::Yes(x) => {{ t = t + 1 }} May::No => {{ t = t + 0 }} }}\n\
         \x20       i = i + 1\n    }}\n    println(\"{{t}}\")\n    return 0\n}}\n"
    );
    refusal_row_full(
        "rc2-140",
        &src,
        &[],
        "E0410",
        &[GENERIC_ENUM_RC],
        &["E0901"],
    );
}

#[test]
fn rc2_141_an_rc_field_of_a_generic_result_read_back_from_a_generic_enum_is_refused() {
    let src = format!(
        "{RC2_LATE_CARRIER}fn main() -> i64 {{\n    let mut i: i64 = 0\n    let mut t: i64 = 0\n\
         \x20   while i < 1000 {{\n        let h = id(H {{ r: Rc::new(N {{ s: victim(i) }}), k: 1 }})\n\
         \x20       let m = May::Yes(h.r)\n        let junk = victim(3)\n\
         \x20       match m {{ May::Yes(x) => {{ let y: Rc<N> = x\n t = t + Rc::get(y).s.len }} \
         May::No => {{ t = t + 0 }} }}\n        i = i + 1\n    }}\n    println(\"{{t}}\")\n\
         \x20   return 0\n}}\n"
    );
    refusal_row_full(
        "rc2-141",
        &src,
        &[],
        "E0410",
        &[GENERIC_ENUM_RC],
        &["E0901"],
    );
}

#[test]
fn rc2_142_an_array_of_rc_fields_of_a_generic_result_is_refused() {
    let src = format!(
        "{RC2_LATE_CARRIER}fn main() -> i64 {{\n    let mut i: i64 = 0\n    let mut t: i64 = 0\n\
         \x20   while i < 1000 {{\n        let h = id(H {{ r: Rc::new(N {{ s: victim(i) }}), k: 1 }})\n\
         \x20       let arr = [h.r, h.r]\n        let junk = victim(3)\n        t = t + arr.len\n\
         \x20       i = i + 1\n    }}\n    println(\"{{t}}\")\n    return 0\n}}\n"
    );
    refusal_row_full(
        "rc2-142",
        &src,
        &[],
        "E0410",
        &["element of array literal is a value of type `Rc<N>` which embeds an `Rc<T>`"],
        &["E0901"],
    );
}

#[test]
fn rc2_143_an_rc_field_of_a_generic_result_pushed_into_a_vec_is_refused() {
    let src = format!(
        "{RC2_LATE_CARRIER}fn main() -> i64 {{\n    let mut i: i64 = 0\n    let mut t: i64 = 0\n\
         \x20   while i < 1000 {{\n        let h = id(H {{ r: Rc::new(N {{ s: victim(i) }}), k: 1 }})\n\
         \x20       let mut v = Vec::new()\n        Vec::push(v, h.r)\n        let junk = victim(3)\n\
         \x20       t = t + v.len\n        i = i + 1\n    }}\n    println(\"{{t}}\")\n    return 0\n}}\n"
    );
    refusal_row_full(
        "rc2-143",
        &src,
        &[],
        "E0410",
        &["element pushed to a Vec has type `Rc<N>` which embeds an `Rc<T>`"],
        &["E0901"],
    );
}

const RC2_144: &str = r#"
fn id<T>(x: T) -> T { return x }
enum May<T> { Yes(T), No }
fn main() -> i64 {
    let m = May::Yes(id(5))
    match m {
        May::Yes(x) => { println("{x}") }
        May::No => { println("no") }
    }
    return 0
}
"#;

#[test]
fn rc2_144_a_plain_value_returned_by_a_generic_still_fills_a_generic_enum() {
    let h = Harness::new();
    h.differential_row("rc2-144", RC2_144, &[], "5\n", (0, 0));
    h.assert_legs(18);
}

#[test]
fn rc2_145_a_slice_of_a_value_whose_type_is_never_inferred_asks_for_an_annotation() {
    refusal_row_full(
        "rc2-145",
        "fn main() -> i64 {\n    let s = fn(t) -> i64 { let x = t[0..1]\n return 0 }\n\
         \x20   return 0\n}\n",
        &[],
        "E0301",
        &["the type of this value is never inferred, so it cannot be sliced"],
        &["E0425", "τ", "E0901"],
    );
}

const RC2_146: &str = r#"
struct N { k: i64 }
struct W { n: Rc<N> }
struct O { w: W }
fn mk(k: i64) -> Rc<N> { return Rc::new(N { k: k }) }
fn mkw(k: i64) -> W { return W { n: mk(k) } }
fn main() -> i64 {
    let mut t = 0
    let mut i = 0
    while i < 1000 {
        let o = O { w: mkw(7) }
        let w = W { n: mk(5) }
        t = t + Rc::get(o.w.n).k + Rc::get(w.n).k
        i = i + 1
    }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_146_a_carrier_field_filled_by_a_call_takes_the_returned_share() {
    let h = Harness::new();
    h.differential_row("rc2-146", RC2_146, &[], "12000\n", (2000, 2000));
    h.lsan_row("rc2-146", RC2_146, &[], "12000\n");
    h.assert_legs(24);
}

const RC2_147: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn build(n: i64) -> Rc<Node> {
    if n == 0 { return Rc::null() }
    return Rc::new(Node { val: n, next: build(n - 1) })
}
fn main() -> i64 {
    let h = build(100)
    let mut t = 0
    t = t + Rc::get(h).val + Rc::get(Rc::get(h).next).val
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_147_a_recursive_constructor_links_each_node_to_the_call_that_built_the_rest() {
    let h = Harness::new();
    h.differential_row("rc2-147", RC2_147, &[], "199\n", (100, 100));
    h.lsan_row("rc2-147", RC2_147, &[], "199\n");
    h.assert_legs(24);
}

const RC2_148: &str = r#"
struct N { k: i64 }
struct W { n: Rc<N> }
struct O { w: W }
fn mk(k: i64) -> Rc<N> { return Rc::new(N { k: k }) }
fn mkw(k: i64) -> W { return W { n: mk(k) } }
fn pick(c: bool) -> i64 {
    let o = O { w: if c { mkw(1) } else { mkw(2) } }
    let p = O { w: { let z = 3; mkw(z) } }
    let q = W { n: if c { mk(4) } else { Rc::new(N { k: 6 }) } }
    return Rc::get(o.w.n).k + Rc::get(p.w.n).k + Rc::get(q.n).k
}
fn main() -> i64 {
    println("{pick(true) + pick(false)}")
    return 0
}
"#;

#[test]
fn rc2_148_a_carrier_field_filled_by_a_conditional_or_a_block_ending_in_a_call() {
    let h = Harness::new();
    h.differential_row("rc2-148", RC2_148, &[], "19\n", (6, 6));
    h.lsan_row("rc2-148", RC2_148, &[], "19\n");
    h.assert_legs(24);
}

const RC2_149: &str = r#"
struct N { k: i64 }
struct W { n: Rc<N> }
struct O { w: W }
enum One { It(W) }
fn id<T>(x: T) -> T { return x }
fn getw(o: &O) -> W { return (*o).w }
fn getn(w: W) -> Rc<N> { return w.n }
fn mkw(k: i64) -> W { return W { n: Rc::new(N { k: k }) } }
fn unone(o: One) -> i64 { return match o { One::It(w) => Rc::get(w.n).k } }
fn main() -> i64 {
    let mut t = 0
    let mut i = 0
    while i < 1000 {
        let w = mkw(3)
        let o1 = O { w: id(w) }
        let o2 = O { w: getw(&o1) }
        let w2 = W { n: getn(w) }
        t = t + Rc::get(o1.w.n).k + Rc::get(o2.w.n).k + Rc::get(w2.n).k + unone(One::It(mkw(1)))
        i = i + 1
    }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_149_a_carrier_field_filled_by_a_generic_or_a_field_returning_call() {
    let h = Harness::new();
    h.differential_row("rc2-149", RC2_149, &[], "10000\n", (2000, 2000));
    h.lsan_row("rc2-149", RC2_149, &[], "10000\n");
    h.assert_legs(24);
}

const RC2_150: &str = r#"
struct N { k: i64 }
fn main() -> i64 {
    let mut t = 0
    let mut i = 0
    while i < 1000 {
        let h: Rc<N> = Rc::null()
        let a = Rc::new(N { k: 1 })
        let b = a
        if Rc::new("abc") == Rc::new("abc") { t = t + 100 }
        if h == Rc::null() { t = t + 1 }
        if a == b { t = t + 1 }
        if a != h { t = t + 1 }
        if Rc::is_null(h) { t = t + 1 }
        if !Rc::is_null(a) { t = t + 1 }
        if Rc::is_null(Rc::new(N { k: 2 })) { t = t + 100 }
        i = i + 1
    }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_150_rc_is_null_and_rc_equality_compare_handles_without_a_share() {
    let h = Harness::new();
    h.differential_row("rc2-150", RC2_150, &[], "5000\n", (4000, 4000));
    h.lsan_row("rc2-150", RC2_150, &[], "5000\n");
    h.assert_legs(24);
}

const RC2_151: &str = r#"
fn main() -> i64 { let a = [1, 2]; let b = [1, 2]; println("{a == b}"); return 0 }
"#;

#[test]
fn rc2_151_equality_on_arrays_is_refused_by_name() {
    refusal_row_full(
        "rc2-151",
        RC2_151,
        &[],
        "E0304",
        &["comparison (`==`) is not supported for `[i64; 2]`"],
        &["E0902"],
    );
}

const RC2_152: &str = r#"
fn main() -> i64 { let x = 1; let y = 1; println("{&x == &y}"); return 0 }
"#;

#[test]
fn rc2_152_equality_on_references_is_refused_by_name() {
    refusal_row_full(
        "rc2-152",
        RC2_152,
        &[],
        "E0304",
        &["comparison (`==`) is not supported for `&i64`"],
        &["E0902"],
    );
}

const RC2_153: &str = r#"
fn g() -> i64 { return 1 }
fn main() -> i64 { let a = g; let b = g; println("{a == b}"); return 0 }
"#;

#[test]
fn rc2_153_equality_on_functions_is_refused_by_name() {
    refusal_row_full(
        "rc2-153",
        RC2_153,
        &[],
        "E0304",
        &["comparison (`==`) is not supported for `() -> i64`"],
        &["E0902"],
    );
}

const RC2_154: &str = r#"
fn main() -> i64 { let a: Vec<i64> = Vec::new(); let b: Vec<i64> = Vec::new(); println("{a == b}"); return 0 }
"#;

#[test]
fn rc2_154_equality_on_vecs_is_refused_by_name() {
    refusal_row_full(
        "rc2-154",
        RC2_154,
        &[],
        "E0304",
        &["comparison (`==`) is not supported for"],
        &["E0902"],
    );
}

const RC2_155: &str = r#"
struct P { a: i64 }
fn id<T>(x: T) -> T { return x }
fn main() -> i64 { let a = id(P { a: 1 }); let b = id([1, 2]); println("{a == a} {b != b}"); return 0 }
"#;

#[test]
fn rc2_155_equality_on_late_typed_structs_and_arrays_is_refused_by_name() {
    refusal_row_full(
        "rc2-155",
        RC2_155,
        &[],
        "E0304",
        &[
            "comparison (`==`) is not supported for `P`",
            "comparison (`!=`) is not supported for `[i64; 2]`",
        ],
        &["E0902"],
    );
}

const RC2_156: &str = r#"
struct N { k: i64 }
fn same<T: eq>(a: T, b: T) -> bool { return a == b }
fn main() -> i64 { let a = Rc::new(N { k: 1 }); println("{same(a, a)}"); return 0 }
"#;

#[test]
fn rc2_156_an_eq_bound_instantiated_with_an_rc_names_the_identity_asymmetry() {
    refusal_row_full(
        "rc2-156",
        RC2_156,
        &[],
        "E0732",
        &["an `Rc` answers `==` by identity, but it is not `eq`"],
        &[],
    );
}

const RC2_157: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn main() -> i64 {
    let mut head: Rc<Node> = Rc::null()
    let mut i = 0
    while i < 1000 {
        head = Rc::new(Node { val: i, next: head })
        i = i + 1
    }
    let mut cur = head
    let mut t = 0
    while !Rc::is_null(cur) {
        t = t + Rc::get(cur).val
        cur = Rc::get(cur).next
    }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_157_a_mutable_rc_binding_builds_and_walks_a_list() {
    let h = Harness::new();
    h.differential_row("rc2-157", RC2_157, &[], "499500\n", (1000, 1000));
    h.lsan_row("rc2-157", RC2_157, &[], "499500\n");
    h.assert_legs(24);
}

const RC2_158: &str = r#"
struct N { k: i64 }
fn f(a: Rc<N>, z: i64) -> i64 { return Rc::get(a).k * 100 + z }
fn main() -> i64 {
    let mut cur = Rc::new(N { k: 1 })
    let r = f(cur, { cur = Rc::new(N { k: 5 }); 0 })
    println("{r} {Rc::get(cur).k}")
    return 0
}
"#;

#[test]
fn rc2_158_a_sibling_that_reassigns_a_mutable_rc_leaves_the_argument_its_old_value() {
    let h = Harness::new();
    h.differential_row("rc2-158", RC2_158, &[], "100 5\n", (6, 6));
    h.lsan_row("rc2-158", RC2_158, &[], "100 5\n");
    h.assert_legs(24);
}

const RC2_159: &str = r#"
struct N { k: i64, s: string }
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut cur = Rc::new(N { k: 42, s: mk(1) })
    let r = &Rc::get(cur).k
    cur = Rc::new(N { k: 7, s: mk(2) })
    let z = Rc::new(N { k: 9, s: mk(3) })
    println("{*r} {Rc::get(z).k}")
    return 0
}
"#;

#[test]
fn rc2_159_a_reference_into_the_payload_of_a_mutable_rc_survives_its_reassignment() {
    let h = Harness::new();
    h.differential_row("rc2-159", RC2_159, &[], "42 9\n", (13, 13));
    h.lsan_row("rc2-159", RC2_159, &[], "42 9\n");
    h.assert_legs(24);
}

const RC2_160: &str = r#"
struct N { s: string }
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut cur = Rc::new(N { s: mk(12345) })
    let b = Rc::get(cur).s.bytes
    cur = Rc::new(N { s: mk(99999) })
    let z = Rc::new(N { s: mk(77777) })
    println("{b[0]} {b[2]} {b.len} {Rc::get(z).s.len}")
    return 0
}
"#;

#[test]
fn rc2_160_a_byte_view_into_the_payload_of_a_mutable_rc_survives_its_reassignment() {
    let h = Harness::new();
    h.differential_row("rc2-160", RC2_160, &[], "97 49 7 7\n", (19, 19));
    h.lsan_row("rc2-160", RC2_160, &[], "97 49 7 7\n");
    h.assert_legs(24);
}

const RC2_161: &str = r#"
struct Node { v: i64, next: Rc<Node> }
struct List { head: Rc<Node>, len: i64 }
fn main() -> i64 {
    let mut l = List { head: Rc::null(), len: 0 }
    let mut i = 0
    while i < 3 { l.head = Rc::new(Node { v: i, next: l.head })
      l.len = l.len + 1
      i = i + 1 }
    println("{l.len} {Rc::get(l.head).v}")
    return 0
}
"#;

#[test]
fn rc2_161_an_rc_field_of_a_local_carrier_is_written_in_place() {
    let h = Harness::new();
    h.differential_row("rc2-161", RC2_161, &[], "3 2\n", (7, 7));
    h.lsan_row("rc2-161", RC2_161, &[], "3 2\n");
    h.assert_legs(24);
}

const RC2_162: &str = r#"
struct Node { v: i64, next: Rc<Node> }
struct List { head: Rc<Node>, len: i64 }
fn push(l: &mut List, v: i64) { (*l).head = Rc::new(Node { v: v, next: (*l).head })
 (*l).len = (*l).len + 1 }
fn main() -> i64 {
    let mut l = List { head: Rc::null(), len: 0 }
    push(&mut l, 1)
    push(&mut l, 2)
    push(&mut l, 3)
    println("{l.len} {Rc::get(l.head).v}")
    return 0
}
"#;

#[test]
fn rc2_162_an_rc_field_is_written_through_a_mutable_reference() {
    let h = Harness::new();
    h.differential_row("rc2-162", RC2_162, &[], "3 3\n", (7, 7));
    h.lsan_row("rc2-162", RC2_162, &[], "3 3\n");
    h.assert_legs(24);
}

const RC2_163: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct List { head: Rc<Node>, len: i64 }
fn main() -> i64 {
    let mut l = List { head: Rc::null(), len: 0 }
    let mut t = 0
    let mut i = 0
    while i < 1000 {
        l.head = Rc::new(Node { val: i, next: l.head })
        l.len = l.len + 1
        if i % 3 == 2 {
            t = t + Rc::get(l.head).val
            l.head = Rc::get(l.head).next
            l.len = l.len - 1
        }
        i = i + 1
    }
    while !Rc::is_null(l.head) {
        t = t + Rc::get(l.head).val
        l.head = Rc::get(l.head).next
    }
    println("{t} {l.len}")
    return 0
}
"#;

#[test]
fn rc2_163_push_and_pop_on_a_local_list_a_thousand_times() {
    let h = Harness::new();
    h.differential_row("rc2-163", RC2_163, &[], "499500 667\n", (1004, 1004));
    h.lsan_row("rc2-163", RC2_163, &[], "499500 667\n");
    h.assert_legs(24);
}

const RC2_164: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct List { head: Rc<Node>, len: i64 }
fn push(l: &mut List, v: i64) {
    (*l).head = Rc::new(Node { val: v, next: (*l).head })
    (*l).len = (*l).len + 1
}
fn pop(l: &mut List) -> i64 {
    let v = Rc::get((*l).head).val
    (*l).head = Rc::get((*l).head).next
    (*l).len = (*l).len - 1
    return v
}
fn main() -> i64 {
    let mut l = List { head: Rc::null(), len: 0 }
    let mut t = 0
    let mut i = 0
    while i < 1000 {
        push(&mut l, i)
        push(&mut l, i + 1)
        t = t + pop(&mut l)
        i = i + 1
    }
    while l.len > 0 { t = t + pop(&mut l) }
    println("{t} {l.len}")
    return 0
}
"#;

#[test]
fn rc2_164_push_and_pop_through_a_mutable_reference_a_thousand_times() {
    let h = Harness::new();
    h.differential_row("rc2-164", RC2_164, &[], "1000000 0\n", (2004, 2004));
    h.lsan_row("rc2-164", RC2_164, &[], "1000000 0\n");
    h.assert_legs(24);
}

const RC2_165: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct List { head: Rc<Node>, len: i64 }
struct O { l: List, k: i64 }
fn f(a: Rc<Node>, z: i64) -> i64 { if Rc::is_null(a) { return z } return Rc::get(a).val * 100 + z }
fn main() -> i64 {
    let mut l = List { head: Rc::new(Node { val: 1, next: Rc::new(Node { val: 2, next: Rc::null() }) }), len: 2 }
    let mut t = 0
    l.head = l.head
    t = t + f(l.head, { l.head = Rc::get(l.head).next; 0 })
    t = t + f(l.head, 0)
    let mut o = O { l: List { head: Rc::null(), len: 0 }, k: 0 }
    o.l.head = Rc::new(Node { val: 5, next: o.l.head })
    o.l.head = Rc::new(Node { val: 6, next: o.l.head })
    t = t + Rc::get(o.l.head).val + Rc::get(Rc::get(o.l.head).next).val
    let h = Rc::new(O { l: List { head: Rc::null(), len: 0 }, k: 0 })
    h.l.head = Rc::new(Node { val: 7, next: h.l.head })
    Rc::get(h).l.head = Rc::new(Node { val: 8, next: Rc::get(h).l.head })
    t = t + Rc::get(h.l.head).val
    let n = Rc::new(Node { val: 9, next: Rc::null() })
    Rc::get(n).next = Rc::new(Node { val: 10, next: Rc::null() })
    Rc::get(n).next = Rc::new(Node { val: 11, next: Rc::get(n).next })
    t = t + Rc::get(Rc::get(Rc::get(n).next).next).val
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_165_rc_field_writes_on_nested_local_handle_and_rc_get_roots() {
    let h = Harness::new();
    h.differential_row("rc2-165", RC2_165, &[], "329\n", (10, 10));
    h.lsan_row("rc2-165", RC2_165, &[], "329\n");
    h.assert_legs(24);
}

const RC2_166: &str = r#"
struct Node { val: i64, s: string, next: Rc<Node> }
struct List { head: Rc<Node>, len: i64 }
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut l = List { head: Rc::new(Node { val: 42, s: mk(12345), next: Rc::null() }), len: 1 }
    let r = &Rc::get(l.head).val
    let b = Rc::get(l.head).s.bytes
    l.head = Rc::new(Node { val: 7, s: mk(99999), next: Rc::null() })
    let z = Rc::new(Node { val: 9, s: mk(77777), next: Rc::null() })
    println("{*r} {b[2]} {b.len} {Rc::get(z).val}")
    return 0
}
"#;

#[test]
fn rc2_166_views_into_the_old_node_survive_an_rc_field_write_on_a_local() {
    let h = Harness::new();
    h.differential_row("rc2-166", RC2_166, &[], "42 49 7 9\n", (19, 19));
    h.lsan_row("rc2-166", RC2_166, &[], "42 49 7 9\n");
    h.assert_legs(24);
}

const RC2_167: &str = r#"
fn mkv<T>(x: T) -> Vec<T> {
    let mut v: Vec<T> = Vec::new()
    Vec::push(v, x)
    return v
}
fn esc<T>(x: T) -> Vec<&i64> {
    let a: [i64; 3] = [7, 8, 41]
    return mkv(&a[2])
}
fn burn() -> i64 {
    let z: [i64; 16] = [5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5, 5]
    return z[3]
}
fn main() -> i64 {
    let v = esc(1)
    let b: i64 = burn()
    println("{*v[0]} {b}")
    return 0
}
"#;

#[test]
fn rc2_167_a_reference_vec_made_inside_a_generic_body_names_the_inner_call() {
    refusal_row_full(
        "rc2-167",
        RC2_167,
        &[],
        "E0714",
        &["the call to `mkv` in `esc` instantiates it with `&i64`"],
        &["the call to `esc` in `main`"],
    );
}

const RC2_168: &str = r#"
struct N { v: i64 }
fn rd<T>(x: &T) -> T { return *x }
fn main() -> i64 {
    let p: Rc<N> = Rc::new(N { v: 9 })
    let q: Rc<N> = rd(&p)
    println("{Rc::get(q).v}  {Rc::get(p).v}")
    return 0
}
"#;

#[test]
fn rc2_168_a_generic_deref_that_yields_an_rc_is_refused_by_name() {
    refusal_row_full(
        "rc2-168",
        RC2_168,
        &[],
        "E0410",
        &[
            "the call to `rd` in `main` instantiates it with `Rc<N>`",
            "out of a `&Rc<N>`",
        ],
        &["E0901"],
    );
}

const RC2_169: &str = r#"
struct N { v: i64 }
fn rd(x: &Rc<N>) -> Rc<N> { return *x }
fn main() -> i64 {
    let p: Rc<N> = Rc::new(N { v: 9 })
    let q: Rc<N> = rd(&p)
    println("{Rc::get(q).v}  {Rc::get(p).v}")
    return 0
}
"#;

#[test]
fn rc2_169_a_concrete_deref_that_yields_an_rc_is_refused_by_name() {
    refusal_row_full(
        "rc2-169",
        RC2_169,
        &[],
        "E0410",
        &["reads an `Rc<N>` out of a reference"],
        &["E0901"],
    );
}

const RC2_170: &str = r#"
struct P { x: i64, s: string }
struct Q { a: i64, t: string }
fn f(r: &P) -> Q { return Q { a: (*r).x, t: (*r).s } }
fn main() -> i64 { let p = P { x: 4, s: "zz" }; let q = f(&p); println("{q.a} {q.t}"); return 0 }
"#;

#[test]
fn rc2_170_fields_read_through_a_reference_fill_a_struct_literal() {
    let h = Harness::new();
    h.differential_row("rc2-170", RC2_170, &[], "4 zz\n", (3, 3));
    h.lsan_row("rc2-170", RC2_170, &[], "4 zz\n");
    h.assert_legs(24);
}

const RC2_171: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct List { head: Rc<Node>, len: i64 }

fn push(l: &mut List, v: i64) {
    (*l).head = Rc::new(Node { val: v, next: (*l).head })
    (*l).len = (*l).len + 1
}

fn pop(l: &mut List) -> i64 {
    if Rc::is_null((*l).head) { return 0 }
    let v = Rc::get((*l).head).val
    (*l).head = Rc::get((*l).head).next
    (*l).len = (*l).len - 1
    return v
}

fn sum(l: &List) -> i64 {
    let mut cur = (*l).head
    let mut t = 0
    while !Rc::is_null(cur) {
        t = t + Rc::get(cur).val
        cur = Rc::get(cur).next
    }
    return t
}

fn build(n: i64) -> Rc<Node> {
    if n == 0 { return Rc::null() }
    return Rc::new(Node { val: n, next: build(n - 1) })
}

fn main() -> i64 {
    let mut l = List { head: build(10), len: 10 }
    let mut t = 0
    let mut i = 0
    while i < 1000 {
        push(&mut l, i % 1000)
        push(&mut l, 1)
        t = t + pop(&mut l) + pop(&mut l)
        if i % 1000 == 0 {
            l.head = build(20)
            l.len = 20
            t = t + sum(&l)
        }
        i = i + 1
    }
    println("{t} {l.len} {sum(&l)}")
    return 0
}
"#;

#[test]
fn rc2_171_the_linked_list_of_the_first_hour() {
    let h = Harness::new();
    h.differential_row("rc2-171", RC2_171, &[], "500710 20 210\n", (2037, 2037));
    h.lsan_row("rc2-171", RC2_171, &[], "500710 20 210\n");
    h.assert_legs(24);
}

const RC2_172: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn find(h: Rc<Node>, v: i64) -> i64 {
    let mut cur = h
    while !Rc::is_null(cur) {
        if Rc::get(cur).val == v { return Rc::get(cur).val * 10 }
        cur = Rc::get(cur).next
    }
    return -1
}
fn main() -> i64 {
    let mut head: Rc<Node> = Rc::null()
    let mut i = 0
    while i < 100 { head = Rc::new(Node { val: i, next: head }); i = i + 1 }
    let mut t = 0
    let mut j = 0
    while j < 100 { t = t + find(head, j); j = j + 1 }
    let mut cur = head
    while true { if Rc::is_null(cur) { break } cur = Rc::get(cur).next }
    println("{t} {find(head, 500)}")
    return 0
}
"#;

#[test]
fn rc2_172_a_mutable_rc_cursor_left_by_return_or_break() {
    let h = Harness::new();
    h.differential_row("rc2-172", RC2_172, &[], "49500 -1\n", (104, 104));
    h.lsan_row("rc2-172", RC2_172, &[], "49500 -1\n");
    h.assert_legs(24);
}

const RC2_173: &str = r#"
nogc fn take(x: &Rc<i64>) -> Rc<i64> { return *x }
fn main() -> i64 { return 0 }
"#;

#[test]
fn rc2_173_a_nogc_deref_of_an_rc_is_still_left_to_the_effect_rule() {
    refusal_row_full(
        "rc2-173",
        RC2_173,
        &[],
        "E0727",
        &["declared nogc"],
        &["reads an `Rc"],
    );
}

const RC2_174: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct List { head: Rc<Node>, len: i64 }
fn main() -> i64 {
    let mut l = List { head: Rc::null(), len: 0 }
    l.head = if true { Rc::new(Node { val: 1, next: Rc::null() }) } else { Rc::null() }
    return 0
}
"#;

#[test]
fn rc2_174_an_indirect_producer_into_a_local_rc_field_is_refused_like_through_a_handle() {
    refusal_row_full(
        "rc2-174",
        RC2_174,
        &[],
        "E0420",
        &["[rc-field-assign]"],
        &["in-place reassignment"],
    );
}

const RC2_175: &str = r#"
struct Node { val: i64, next: Rc<Node> }
struct List { head: Rc<Node>, len: i64 }
fn f(mut l: List) -> i64 {
    l.head = Rc::new(Node { val: 3, next: Rc::null() })
    return Rc::get(l.head).val
}
fn g(mut h: Rc<Node>) -> i64 {
    h = Rc::new(Node { val: 4, next: Rc::null() })
    return Rc::get(h).val
}
fn main() -> i64 {
    let l = List { head: Rc::new(Node { val: 1, next: Rc::null() }), len: 1 }
    let a = f(l)
    let b = f(l)
    let h = Rc::new(Node { val: 2, next: Rc::null() })
    let c = g(h)
    let d = g(h)
    let z = Rc::new(Node { val: 9, next: Rc::null() })
    println("{a} {b} {c} {d} {Rc::get(l.head).val} {Rc::get(h).val} {Rc::get(z).val}")
    return 0
}
"#;

#[test]
fn rc2_175_a_mutable_parameter_rewritten_leaves_the_caller_its_value() {
    let h = Harness::new();
    h.differential_row("rc2-175", RC2_175, &[], "3 3 4 4 1 2 9\n", (26, 26));
    h.lsan_row("rc2-175", RC2_175, &[], "3 3 4 4 1 2 9\n");
    h.assert_legs(24);
}

const RC2_176: &str = r#"
struct N { val: i64 }
fn adv(r: &mut Rc<N>) { *r = Rc::new(N { val: 77 }) }
fn main() -> i64 {
    let mut cur = Rc::new(N { val: 1 })
    adv(&mut cur)
    println("{Rc::get(cur).val}")
    return 0
}
"#;

#[test]
fn rc2_176_an_rc_written_through_a_mutable_reference_is_refused_by_name() {
    refusal_row_full(
        "rc2-176",
        RC2_176,
        &[],
        "E0410",
        &["writes an `Rc<N>` through a reference"],
        &["E0901"],
    );
}

const RC2_177: &str = r#"
struct N { val: i64 }
fn adv(r: &mut Rc<N>) { *r = Rc::new(N { val: 77 }) }
fn g(mut h: Rc<N>) -> i64 { adv(&mut h)
  return Rc::get(h).val }
fn main() -> i64 {
    let a = Rc::new(N { val: 1 })
    println("{g(a)} {Rc::get(a).val}")
    return 0
}
"#;

#[test]
fn rc2_177_an_rc_written_through_a_reference_to_a_mutable_parameter_is_refused_by_name() {
    refusal_row_full(
        "rc2-177",
        RC2_177,
        &[],
        "E0410",
        &["writes an `Rc<N>` through a reference"],
        &["E0901"],
    );
}

const RC2_178: &str = r#"
struct N { val: i64 }
fn set(r: &mut Rc<N>, v: Rc<N>) { *r = v }
fn main() -> i64 {
 let mut cur = Rc::new(N { val: 1 })
 let b = Rc::new(N { val: 2 })
 set(&mut cur, b)
 println("{Rc::get(cur).val} {Rc::get(b).val}")
 return 0 }
"#;

#[test]
fn rc2_178_an_rc_argument_written_through_a_mutable_reference_is_refused_by_name() {
    refusal_row_full(
        "rc2-178",
        RC2_178,
        &[],
        "E0410",
        &["writes an `Rc<N>` through a reference"],
        &["E0901"],
    );
}

const RC2_179: &str = r#"
struct Node { val: i64, s: string, next: Rc<Node> }
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn nd(v: i64, nx: Rc<Node>) -> Rc<Node> { return Rc::new(Node { val: v, s: mk(v), next: nx }) }

fn adv(r: &mut Rc<Node>) { *r = nd(77, Rc::null()) }
fn main() -> i64 {
    let mut t = 0
    let mut i = 0
    while i < 1000 {
        let mut cur = nd(1, nd(2, Rc::null()))
        adv(&mut cur)
        t = t + Rc::get(cur).val + Rc::get(cur).s.len
        i = i + 1
    }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_179_a_list_cursor_advanced_through_a_mutable_reference_is_refused_by_name() {
    refusal_row_full(
        "rc2-179",
        RC2_179,
        &[],
        "E0410",
        &["writes an `Rc<Node>` through a reference"],
        &["E0901"],
    );
}

const RC2_180: &str = r#"
struct N { val: i64 }
fn set<T>(r: &mut T, v: T) { *r = v }
fn main() -> i64 {
 let mut cur = Rc::new(N { val: 1 })
 let b = Rc::new(N { val: 2 })
 set(&mut cur, b)
 println("{Rc::get(cur).val} {Rc::get(b).val}")
 return 0 }
"#;

#[test]
fn rc2_180_a_generic_write_through_a_reference_at_an_rc_says_it_writes() {
    refusal_row_full(
        "rc2-180",
        RC2_180,
        &[],
        "E0410",
        &[
            "the call to `set` in `main` instantiates it with `Rc<N>`",
            "writes an `Rc<N>` through a `&mut Rc<N>`",
        ],
        &["reads an"],
    );
}

const RC2_181: &str = r#"
struct N { k: i64 }
struct W { n: Rc<N> }
struct O { w: W, k: i64 }
fn main() -> i64 { let mut o = O { w: W { n: Rc::new(N{k:1}) }, k: 0 }
 o.w = W { n: Rc::new(N{k:2}) }
 println("{Rc::get(o.w.n).k}")
 return 0 }
"#;

#[test]
fn rc2_181_a_nominal_carrier_field_written_in_place_is_still_refused() {
    refusal_row_full(
        "rc2-181",
        RC2_181,
        &[],
        "E0410",
        &["in-place reassignment of field `w` (type `W`"],
        &[],
    );
}

const RC2_182: &str = r#"
struct N { k: i64 }
struct L { head: Rc<N>, k: i64 }
nogc fn f(mut l: L) -> i64 { l.head = Rc::null()
  return l.k }
fn main() -> i64 { let l = L { head: Rc::new(N{k:1}), k: 4 }
  println("{f(l)} {Rc::get(l.head).k}")
  return 0 }
"#;

#[test]
fn rc2_182_an_rc_field_written_in_a_nogc_function_is_still_refused() {
    refusal_row_full(
        "rc2-182",
        RC2_182,
        &[],
        "E0410",
        &["in-place reassignment of field `head` (type `Rc<N>`"],
        &[],
    );
}

const RC2_183: &str = r#"
fn mkf() -> fn() -> i64 {
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 41)
    return fn() -> i64 {
        let s = Vec::as_slice(v)
        for i in 0..1000 { Vec::push(v, i) }
        return s[0]
    }
}
fn main() -> i64 {
    let f = mkf()
    println("{f()}")
    return 0
}
"#;

#[test]
fn rc2_183_a_view_of_a_captured_vec_held_across_a_push_in_the_closure_is_refused() {
    refusal_row_full(
        "rc2-183",
        RC2_183,
        &[],
        "E0711",
        &["cannot write to `v` while it is borrowed"],
        &[],
    );
}

const RC2_184: &str = r#"
fn mkf() -> fn() -> i64 {
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 41)
    return fn() -> i64 {
        let s = Vec::as_slice(v)
        v = Vec::new()
        for i in 0..1000 { Vec::push(v, i) }
        return s[0]
    }
}
fn main() -> i64 {
    let f = mkf()
    println("{f()}")
    return 0
}
"#;

#[test]
fn rc2_184_a_view_of_a_captured_vec_held_across_its_reassignment_in_the_closure_is_refused() {
    refusal_row_full(
        "rc2-184",
        RC2_184,
        &[],
        "E0711",
        &["cannot write to `v` while it is borrowed"],
        &[],
    );
}

const RC2_185: &str = r#"
fn main() -> i64 {
    let f = fn() -> i64 {
        let mut v: Vec<i64> = Vec::new()
        Vec::push(v, 41)
        let s = Vec::as_slice(v)
        for i in 0..1000 { Vec::push(v, i) }
        return s[0]
    }
    println("{f()}")
    return 0
}
"#;

#[test]
fn rc2_185_a_view_of_a_vec_local_to_a_closure_held_across_a_push_is_refused() {
    refusal_row_full(
        "rc2-185",
        RC2_185,
        &[],
        "E0711",
        &["cannot write to `v` while it is borrowed"],
        &[],
    );
}

const RC2_186: &str = r#"
fn main() -> i64 {
    let mut w: Vec<i64> = Vec::new()
    Vec::push(w, 41)
    let f = fn(mut v: Vec<i64>) -> i64 {
        let s = Vec::as_slice(v)
        v = Vec::new()
        for i in 0..1000 { Vec::push(v, i) }
        return s[0]
    }
    println("{f(w)}")
    return 0
}
"#;

#[test]
fn rc2_186_a_view_of_a_closure_parameter_vec_held_across_its_reassignment_is_refused() {
    refusal_row_full(
        "rc2-186",
        RC2_186,
        &[],
        "E0711",
        &["cannot write to `v` while it is borrowed"],
        &[],
    );
}

const RC2_187: &str = r#"
fn mkf() -> fn() -> i64 {
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 41)
    return fn() -> i64 {
        let g = fn() -> i64 {
            let s = Vec::as_slice(v)
            for i in 0..1000 { Vec::push(v, i) }
            return s[0]
        }
        return g()
    }
}
fn main() -> i64 {
    let f = mkf()
    println("{f()}")
    return 0
}
"#;

#[test]
fn rc2_187_a_view_of_a_captured_vec_in_a_nested_closure_held_across_a_push_is_refused() {
    refusal_row_full(
        "rc2-187",
        RC2_187,
        &[],
        "E0711",
        &["cannot write to `v` while it is borrowed"],
        &[],
    );
}

const RC2_188: &str = r#"
fn mkf() -> fn() -> i64 {
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 41)
    return fn() -> i64 {
        let mut t = 0
        for x in Vec::as_slice(v) {
            for i in 0..1000 { Vec::push(v, i) }
            t = t + x
        }
        return t
    }
}
fn main() -> i64 {
    let f = mkf()
    println("{f()}")
    return 0
}
"#;

#[test]
fn rc2_188_a_loop_over_a_captured_vec_slice_that_pushes_is_refused() {
    refusal_row_full(
        "rc2-188",
        RC2_188,
        &[],
        "E0711",
        &["cannot write to `v` while it is borrowed"],
        &[],
    );
}

const RC2_189: &str = r#"
fn mkf() -> fn() -> i64 {
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 41)
    return fn() -> i64 {
        let s: &mut [i64] = Vec::try_as_unique_mut_slice(v)
        for i in 0..1000 { Vec::push(v, i) }
        return s[0]
    }
}
fn main() -> i64 {
    let f = mkf()
    println("{f()}")
    return 0
}
"#;

#[test]
fn rc2_189_a_mutable_view_of_a_captured_vec_held_across_a_push_is_refused() {
    refusal_row_full(
        "rc2-189",
        RC2_189,
        &[],
        "E0711",
        &["cannot write to `v` while it is borrowed"],
        &[],
    );
}

const RC2_190: &str = r#"
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn mkf() -> fn() -> i64 {
    let mut s = mk(1)
    return fn() -> i64 {
        let b = s.bytes
        s = mk(2)
        return b[0] as i64
    }
}
fn main() -> i64 {
    let f = mkf()
    println("{f()}")
    return 0
}
"#;

#[test]
fn rc2_190_the_bytes_of_a_captured_string_held_across_its_reassignment_are_refused() {
    refusal_row_full(
        "rc2-190",
        RC2_190,
        &[],
        "E0711",
        &["cannot write to `s` while it is borrowed"],
        &[],
    );
}

const RC2_191: &str = r#"
struct B { s: string, k: i64 }
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn mkf() -> fn() -> i64 {
    let mut bb = B { s: mk(1), k: 0 }
    return fn() -> i64 {
        let b = bb.s.bytes
        bb.s = mk(2)
        return b[0] as i64
    }
}
fn main() -> i64 {
    let f = mkf()
    println("{f()}")
    return 0
}
"#;

#[test]
fn rc2_191_the_bytes_of_a_captured_struct_field_held_across_a_field_store_are_refused() {
    refusal_row_full(
        "rc2-191",
        RC2_191,
        &[],
        "E0711",
        &["cannot write to `bb.s` while it is borrowed"],
        &[],
    );
}

const RC2_192: &str = r#"
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn mkf() -> fn() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, mk(1))
    return fn() -> i64 {
        let b = v[0].bytes
        v[0] = mk(2)
        return b[0] as i64
    }
}
fn main() -> i64 {
    let f = mkf()
    println("{f()}")
    return 0
}
"#;

#[test]
fn rc2_192_the_bytes_of_a_captured_vec_element_held_across_an_element_store_are_refused() {
    refusal_row_full(
        "rc2-192",
        RC2_192,
        &[],
        "E0711",
        &["cannot write to `v[..]` while it is borrowed"],
        &[],
    );
}

const RC2_193: &str = r#"
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn mkf() -> fn() -> i64 {
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 41)
    return fn() -> i64 {
        let s = Vec::as_slice(v)
        let g = fn() -> i64 {
            for i in 0..1000 { Vec::push(v, i) }
            return v.len
        }
        let n = g()
        return s[0] + n
    }
}
fn main() -> i64 {
    let f = mkf()
    let a = f()
    let b = f()
    let mut w: Vec<string> = Vec::new()
    Vec::push(w, mk(1))
    let s = Vec::as_slice(w)
    let h = fn() -> i64 {
        for i in 0..100 { Vec::push(w, mk(i)) }
        return w.len
    }
    let n = h()
    println("{a} {b} {s[0]} {n}")
    return 0
}
"#;

#[test]
fn rc2_193_a_captured_vec_viewed_without_a_conflicting_write_stays_accepted() {
    let h = Harness::new();
    h.differential_row("rc2-193", RC2_193, &[], "1042 1042 ab1 101\n", (220, 220));
    h.lsan_row("rc2-193", RC2_193, &[], "1042 1042 ab1 101\n");
    h.assert_legs(24);
}

const RC2_194: &str = r#"
fn mk(n: i64) -> string { return "s" + "{n}" }
fn mkf() -> fn() -> i64 {
    let mut v: Vec<string> = Vec::new()
    for i in 0..3 { Vec::push(v, mk(i)) }
    return fn() -> i64 { return v.len }
}
fn main() -> i64 {
    let mut t = 0
    for i in 0..1000 { let f = mkf()
        t = t + f() }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_194_closures_returning_a_captured_vec_length_stay_balanced() {
    let h = Harness::new();
    h.differential_row("rc2-194", RC2_194, &[], "3000\n", (8000, 8000));
    h.lsan_row("rc2-194", RC2_194, &[], "3000\n");
    h.assert_legs(24);
}

const RC2_195: &str = r#"
fn mk(n: i64) -> string { return "s" + "{n}" }
fn main() -> i64 {
    let mut t = 0
    for k in 0..1000 {
        let mut v: Vec<string> = Vec::new()
        Vec::push(v, mk(1))
        let f = fn() -> i64 { Vec::push(v, mk(2))
            return v.len }
        let a = f()
        Vec::push(v, mk(3))
        let b = f()
        let g = f
        let c = g()
        t = t + a * 100 + b * 10 + c + v.len * 1000
    }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_195_a_closure_pushing_into_its_captured_vec_stays_balanced() {
    let h = Harness::new();
    h.differential_row("rc2-195", RC2_195, &[], "2234000\n", (13000, 13000));
    h.lsan_row("rc2-195", RC2_195, &[], "2234000\n");
    h.assert_legs(24);
}

const RC2_196: &str = r#"
fn mk(n: i64) -> string { return "item" + "{n}" }
fn add(items: &mut Vec<string>, total: &mut i64, x: string) {
    Vec::push(*items, x)
    *total = *total + 1
}
fn main() -> i64 {
    let mut t = 0
    for k in 0..1000 {
        let mut items: Vec<string> = Vec::new()
        let mut total = 0
        for i in 0..3 { add(&mut items, &mut total, mk(i)) }
        let snap = items
        add(&mut items, &mut total, mk(9))
        t = t + items.len + snap.len + total
    }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_196_two_vec_locals_passed_by_mutable_reference_replace_a_vec_field() {
    let h = Harness::new();
    h.differential_row("rc2-196", RC2_196, &[], "11000\n", (10000, 10000));
    h.lsan_row("rc2-196", RC2_196, &[], "11000\n");
    h.assert_legs(24);
}

const RC2_197: &str = r#"
struct Bag { items: [string; 8], n: i64 }
fn mk(n: i64) -> string { return "item" + "{n}" }
fn add(b: &mut Bag, x: string) {
    (*b).items[(*b).n] = x
    (*b).n = (*b).n + 1
}
fn main() -> i64 {
    let mut t = 0
    for k in 0..1000 {
        let mut b = Bag { items: ["", "", "", "", "", "", "", ""], n: 0 }
        for i in 0..3 { add(&mut b, mk(i)) }
        let c = b
        add(&mut b, mk(9))
        t = t + b.n + c.n + b.items[3].len + c.items[3].len
    }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_197_a_fixed_size_array_field_replaces_a_vec_field() {
    let h = Harness::new();
    h.differential_row("rc2-197", RC2_197, &[], "12000\n", (8000, 8000));
    h.lsan_row("rc2-197", RC2_197, &[], "12000\n");
    h.assert_legs(24);
}

const RC2_198: &str = r#"
enum E { A(Vec<string>), B }
fn main() -> i64 { return 0 }
"#;

#[test]
fn rc2_198_a_vec_enum_payload_names_the_own_binding_workaround() {
    refusal_row_full(
        "rc2-198",
        RC2_198,
        &[],
        "E0902",
        &[
            "variant `A` of enum `E` has a payload of type `Vec<string>`: a Vec inside an enum is \
             not supported in 0.1; keep the Vec in its own binding and pass it with `&mut`",
        ],
        &["struct or enum"],
    );
}

const RC2_199: &str = r#"
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn grow(v: &mut Vec<i64>) { for i in 0..1000 { Vec::push(*v, i) } }
fn id(x: i64) -> i64 { return x }
fn getf(v: &mut Vec<i64>) -> fn(i64) -> i64 {
    for i in 0..1000 { Vec::push(*v, i) }
    return fn(x: i64) -> i64 { return x }
}
fn main() -> i64 {
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 41)
    let r = Vec::as_slice(v)
    let z = getf(&mut v)(0)
    println("{r[0]} {z}")
    return 0
}
"#;

#[test]
fn rc2_199_a_callee_expression_that_borrows_mutably_while_a_view_lives_is_refused() {
    refusal_row_full(
        "rc2-199",
        RC2_199,
        &[],
        "E0713",
        &["cannot borrow `v` as mutable while it is already borrowed"],
        &[],
    );
}

const RC2_200: &str = r#"
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn grow(v: &mut Vec<i64>) { for i in 0..1000 { Vec::push(*v, i) } }
fn id(x: i64) -> i64 { return x }
fn main() -> i64 {
    let mut s = mk(1)
    let r = s.bytes
    let f = fn(x: i64) -> i64 { return x }
    let z = (if true { s = mk(2) + mk(3)
        f } else { f })(0)
    println("{r[0]} {z}")
    return 0
}
"#;

#[test]
fn rc2_200_a_callee_if_expression_that_reassigns_a_viewed_string_is_refused() {
    refusal_row_full(
        "rc2-200",
        RC2_200,
        &[],
        "E0711",
        &["cannot write to `s` while it is borrowed"],
        &[],
    );
}

const RC2_201: &str = r#"
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn grow(v: &mut Vec<i64>) { for i in 0..1000 { Vec::push(*v, i) } }
fn id(x: i64) -> i64 { return x }
fn main() -> i64 {
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 41)
    let r = Vec::as_slice(v)
    let f = fn(x: i64) -> i64 { return x }
    let z = { for i in 0..1000 { Vec::push(v, i) }
        f }(0)
    println("{r[0]} {z}")
    return 0
}
"#;

#[test]
fn rc2_201_a_callee_block_that_pushes_into_a_viewed_vec_is_refused() {
    refusal_row_full(
        "rc2-201",
        RC2_201,
        &[],
        "E0711",
        &["cannot write to `v` while it is borrowed"],
        &[],
    );
}

const RC2_202: &str = r#"
fn id(x: i64) -> i64 { return x }
fn main() -> i64 {
    let k = fn() -> i64 {
        let mut v: Vec<i64> = Vec::new()
        Vec::push(v, 41)
        let r = Vec::as_slice(v)
        let f = fn(x: i64) -> i64 { return x }
        let z = { for i in 0..1000 { Vec::push(v, i) }
            f }(0)
        return r[0] + z
    }
    println("{k()}")
    return 0
}
"#;

#[test]
fn rc2_202_a_callee_block_that_pushes_into_a_viewed_vec_inside_a_closure_is_refused() {
    refusal_row_full(
        "rc2-202",
        RC2_202,
        &[],
        "E0711",
        &["cannot write to `v` while it is borrowed"],
        &[],
    );
}

const RC2_203: &str = r#"
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn pick(b: bool) -> fn(i64) -> i64 {
    if b { return fn(x: i64) -> i64 { return x + 1 } }
    return fn(x: i64) -> i64 { return x * 2 }
}
fn main() -> i64 {
    let mut t = 0
    for k in 0..100 {
        let mut v: Vec<string> = Vec::new()
        Vec::push(v, mk(k))
        let mut w: Vec<i64> = Vec::new()
        let r = Vec::as_slice(v)
        let f = fn(x: i64) -> i64 { return x + 10 }
        let a = pick(true)(r[0].len)
        let b = (if r[0].len > 3 { f } else { pick(false) })(3)
        let c = { Vec::push(w, 5)
            f }(w.len)
        t = t + a + b + c + r[0].len
    }
    println("{t}")
    return 0
}
"#;

#[test]
fn rc2_203_complex_callee_expressions_without_a_conflict_stay_accepted() {
    let h = Harness::new();
    h.differential_row("rc2-203", RC2_203, &[], "3210\n", (400, 400));
    h.lsan_row("rc2-203", RC2_203, &[], "3210\n");
    h.assert_legs(24);
}

const RC2_204: &str = r#"
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn grow(v: &mut Vec<i64>) { for i in 0..1000 { Vec::push(*v, i) } }
fn id(x: i64) -> i64 { return x }
let F = fn() -> i64 {
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 41)
    let r = Vec::as_slice(v)
    for i in 0..1000 { Vec::push(v, i) }
    return r[0]
}
fn main() -> i64 {
    println("{F()}")
    return 0
}
"#;

#[test]
fn rc2_204_a_view_in_a_closure_bound_at_file_level_held_across_a_push_is_refused() {
    refusal_row_full(
        "rc2-204",
        RC2_204,
        &[],
        "E0711",
        &["cannot write to `v` while it is borrowed"],
        &[],
    );
}

const RC2_205: &str = r#"
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn grow(v: &mut Vec<i64>) { for i in 0..1000 { Vec::push(*v, i) } }
fn id(x: i64) -> i64 { return x }
fn outer() -> i64 {
    fn inner() -> i64 {
        let f = fn() -> i64 {
            let mut v: Vec<i64> = Vec::new()
            Vec::push(v, 41)
            let r = Vec::as_slice(v)
            for i in 0..1000 { Vec::push(v, i) }
            return r[0]
        }
        return f()
    }
    return inner()
}
fn main() -> i64 {
    println("{outer()}")
    return 0
}
"#;

#[test]
fn rc2_205_a_view_in_a_closure_inside_a_nested_function_held_across_a_push_is_refused() {
    refusal_row_full(
        "rc2-205",
        RC2_205,
        &[],
        "E0711",
        &["cannot write to `v` while it is borrowed"],
        &[],
    );
}

const RC2_206: &str = r#"
fn mk(n: i64) -> string { return "ab" + "{n}" }
fn take(s: string) -> i64 { return s.len }
struct B { s: string, k: i64 }
fn main() -> i64 {
    let f = fn(a: &[i64]) -> fn() -> i64 {
        return fn() -> i64 { return a[0] }
    }
    let mut g = fn() -> i64 { return 0 }
    if true {
        let mut v: Vec<i64> = Vec::new()
        Vec::push(v, 41)
        g = f(Vec::as_slice(v))
    }
    let mut w: Vec<i64> = Vec::new()
    for i in 0..1000 { Vec::push(w, 7) }
    println("{g()}")
    return 0
}
"#;

#[test]
fn rc2_206_a_closure_capturing_a_slice_parameter_of_another_closure_is_refused() {
    refusal_row_full(
        "rc2-206",
        RC2_206,
        &[],
        "E0725",
        &["a closure that captures a reference is not supported"],
        &[],
    );
}

const RC2_207: &str = r#"
struct Node { val: i64, next: Rc<Node> }
fn run() -> i64 {
    let a: Rc<Node> = Rc::new(Node { val: 1, next: Rc::null() })
    let b: Rc<Node> = Rc::new(Node { val: 2, next: Rc::null() })
    a.next = b
    let v = vec[10, 20, 30]
    let w = v
    Vec::push(w, 40)
    __aelys_collect()
    return Rc::get(a).val + Rc::get(Rc::get(a).next).val + w[3] + v[0]
}
fn main() -> i64 {
    println("{run()}")
    return 0
}
"#;

#[test]
fn rc2_207_an_rc_linked_by_a_field_write_beside_a_copied_vec_is_released() {
    let h = Harness::new();
    h.differential_row("rc2-207", RC2_207, &[], "53\n", (4, 4));
    h.lsan_row("rc2-207", RC2_207, &[], "53\n");
    h.assert_legs(24);
}

const RC2_208: &str = r#"
needs std.str
needs std.vec
needs std.slice
needs std.sort
needs std.math
needs std.result
needs std.io

struct Entry { key: string, score: Rc<i64> }
struct Node { val: i64, next: Rc<Node> }
struct List { head: Rc<Node>, len: i64 }
struct Tags { names: [string; 4], n: i64 }

fn line_for(i: i64) -> string {
    return "  id" + str.from_int(i % 37) + " , " + str.from_int((i * 7) % 101) + " , x" + str.from_int(i % 5) + "  "
}

fn score_of(e: Entry) -> i64 {
    return Rc::get(e.score)
}

fn mk_entry(key: string, value: i64) -> Entry {
    return Entry { key: key, score: Rc::new(value) }
}

fn push(l: &mut List, v: i64) {
    (*l).head = Rc::new(Node { val: v, next: (*l).head })
    (*l).len = (*l).len + 1
}

fn pop(l: &mut List) -> i64 {
    if Rc::is_null((*l).head) { return 0 }
    let v = Rc::get((*l).head).val
    (*l).head = Rc::get((*l).head).next
    (*l).len = (*l).len - 1
    return v
}

fn walk(l: &List) -> i64 {
    let mut cur = (*l).head
    let mut t = 0
    while !Rc::is_null(cur) {
        t = t + Rc::get(cur).val
        cur = Rc::get(cur).next
    }
    return t
}

fn record(seen: &mut Vec<string>, count: &mut i64, s: string) {
    Vec::push(*seen, s)
    *count = *count + 1
}

fn tag_add(t: &mut Tags, s: string) {
    (*t).names[(*t).n % 4] = s
    (*t).n = (*t).n + 1
}

fn main() -> i64 {
    let mut total: i64 = 0
    let mut l = List { head: Rc::null(), len: 0 }
    push(&mut l, 5)
    push(&mut l, 6)
    let mut i: i64 = 0
    while i < 300 {
        let raw: string = line_for(i)
        let clean: string = str.trim(raw)
        let parts: Vec<string> = str.split(clean, ",")
        let key: string = str.trim(parts[0])
        let value: i64 = result.unwrap_or(str.parse_int(str.trim(parts[1])), 0)
        let tag: string = str.substring(str.trim(parts[2]), 0, 2)
        let e: Entry = mk_entry(key, value)
        let mut names: Vec<string> = Vec::new()
        Vec::push(names, tag)
        Vec::push(names, e.key)
        Vec::push(names, "zz")
        let mut view: &mut [string] = Vec::try_as_unique_mut_slice(names)
        sort.strings(view)
        let joined: string = str.join(&names, "|")
        let mut nums: Vec<i64> = Vec::new()
        Vec::push(nums, value)
        Vec::push(nums, math.gcd(value, 12))
        Vec::push(nums, math.clamp(value - 50, 0, 9))
        let k: i64 = joined.len
        let shifted: Vec<i64> = vec.map(&nums, fn (x: i64) -> i64 { return x + k })
        let kept: Vec<i64> = vec.filter(&shifted, fn (x: i64) -> bool { return x % 2 == 0 })
        let acc: i64 = vec.fold(&kept, 0, fn (a: i64, b: i64) -> i64 { return a + b })
        let sorted: Vec<i64> = vec.sorted(&shifted)
        let first: i64 = result.some_or(slice.first(Vec::as_slice(sorted)), 0)
        let hit: i64 = if str.contains(joined, "id1") { 1 } else { 0 }
        let ch: i64 = str.char_count(joined) + str.index_of(joined, "|") + str.compare(tag, "x2")
        total = total + score_of(e) + acc + first + hit + ch + math.abs(math.sign(value - 50)) + vec.sum(&nums) % 7
        push(&mut l, value)
        let top = l.head
        push(&mut l, k)
        let moved: i64 = if top == l.head { 0 } else { 1 }
        let popped: i64 = pop(&mut l)
        let back: i64 = if top == l.head { 1 } else { 0 }
        total = total + moved + back + popped + pop(&mut l) + l.len
        if i % 50 == 0 { total = total + walk(&l) }
        let mut seen: Vec<string> = Vec::new()
        let mut count: i64 = 0
        record(&mut seen, &mut count, tag)
        record(&mut seen, &mut count, joined)
        let peek = fn () -> i64 {
            let s = Vec::as_slice(seen)
            return s.len + s[1].len
        }
        let lead = fn () -> i64 {
            let b = joined.bytes
            return b[0] + b.len
        }
        let mut tg = Tags { names: ["", "", "", ""], n: 0 }
        tag_add(&mut tg, key)
        tag_add(&mut tg, tag)
        tag_add(&mut tg, str.from_int(value))
        total = total + peek() + lead() + count + tg.n + tg.names[2].len + tg.names[0].len
        i = i + 1
    }
    io.print_out(str.from_int(total) + " " + str.from_int(walk(&l)) + "\n")
    return 0
}
"#;

#[test]
fn rc2_208_an_ordinary_program_over_the_whole_library_and_the_rc_list_surface_is_balanced() {
    let h = Harness::new();
    h.differential_row("rc2-208", RC2_208, &[], "96823 11\n", (10807, 10807));
    h.lsan_row("rc2-208", RC2_208, &[], "96823 11\n");
    h.assert_legs(24);
}

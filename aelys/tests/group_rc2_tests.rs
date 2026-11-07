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

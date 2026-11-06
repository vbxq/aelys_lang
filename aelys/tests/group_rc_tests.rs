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
        .expect("the aelys package must sit inside the workspace root")
        .to_path_buf()
}

fn library_root() -> PathBuf {
    let dir = repo_root().join("std");
    for module in MODULES {
        let file = dir.join(module);
        let size = fs::metadata(&file)
            .unwrap_or_else(|e| panic!("std/{module} must be readable at {}: {e}", file.display()))
            .len();
        assert!(size > 0, "std/{module} must not be empty");
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

    // `needs std.str` resolves under the root file, so the library is copied beside every root
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
                panic!("{id} at {tag} must compile:\n{err}");
            }
        }
        let exe = exe_path_for(root);
        exe.is_file().then_some(exe)
    }

    // leak never frees, so its answer is right by construction and any divergence is an early free
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
            // the allocator is an env var, so the two legs of a level share one compiled binary
            for (alloc_name, alloc) in ALLOCATORS {
                let mut answers: Vec<String> = Vec::new();
                let mut seen_counts: Vec<(&str, (i64, i64))> = Vec::new();
                for (runtime, variant) in VARIANTS {
                    let leg = format!("{tag}_{alloc_name}_{runtime}");
                    let root = self.stage(id, &leg, src, files);
                    let Some(exe) = self.compile_at(id, &leg, &root, *opt, *variant) else {
                        assert!(
                            linker_skip_declared(),
                            "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped \
                             differential carries no runtime evidence at all"
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
                    // leak carries the release guard too, so a nonzero code can be a double release
                    assert_eq!(
                        exit_code(&out.status),
                        0,
                        "{id} at {tag}/{alloc_name}/{runtime} must exit 0\nstderr:\n{seen_err}"
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
                // the divergence is asserted before the counts: the counts read 6/6 through a double free
                for (i, (runtime, _)) in VARIANTS.iter().enumerate().skip(1) {
                    assert_eq!(
                        answers[i], answers[0],
                        "{id} at {tag}/{alloc_name}: {runtime} answers {:?} where leak answers \
                         {:?}; leak never frees, so a divergence is a release emitted before the \
                         last reader, and no counter and no exit code can see it",
                        answers[i], answers[0]
                    );
                }
                assert_eq!(
                    answers[0], stdout,
                    "{id} at {tag}/{alloc_name}: the leak runtime MUST answer {stdout:?}"
                );
                for (runtime, got) in &seen_counts {
                    assert_eq!(
                        *got, counted,
                        "{id} at {tag}/{alloc_name}/{runtime}: MUST be allocs={} frees={}",
                        counted.0, counted.1
                    );
                }
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
             is a confident zero"
        );
    }
}

static ASAN_DIR: OnceLock<PathBuf> = OnceLock::new();

fn asan_archives() -> PathBuf {
    ASAN_DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("aelys-group-rc-asan-{}", std::process::id()));
        fs::create_dir_all(&dir).expect("asan dir");
        let core = repo_root().join("core").join("src");
        for (lib, rc) in [("core-rc", "aelys_rc_real.c"), ("core-leak", "aelys_rc_leak.c")] {
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
                    .expect("clang is required for the lsan rows");
                assert!(out.status.success(), "{unit}: {}", String::from_utf8_lossy(&out.stderr));
                objects.push(obj);
            }
            let out = Command::new("ar")
                .arg("rcs")
                .arg(dir.join(format!("lib{lib}.a")))
                .args(&objects)
                .output()
                .expect("ar");
            assert!(out.status.success(), "ar: {}", String::from_utf8_lossy(&out.stderr));
        }
        dir
    })
    .clone()
}

impl Harness {
    // malloc, so the blocks lsan scans are the ones the program freed or kept, unlike immix
    fn lsan_run(&self, id: &str, leg: &str, src: &str, files: &[(&str, &str)], opt: OptimizationLevel, variant: RuntimeVariant) -> (i32, String, String) {
        let root = self.stage(id, leg, src, files);
        compile_file_with_llvm_variant(&root, opt, false, variant)
            .unwrap_or_else(|e| panic!("{id} at {leg} must compile:\n{e}"));
        let object = root.with_extension("o");
        assert!(object.is_file(), "{id} at {leg}: the driver left no object");
        let lib = if matches!(variant, RuntimeVariant::Leak) { "core-leak" } else { "core-rc" };
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
        assert!(link.status.success(), "{id}: link failed:\n{}", String::from_utf8_lossy(&link.stderr));
        let out = Command::new(&exe)
            .env("AELYS_ALLOC", "malloc")
            .env("ASAN_OPTIONS", "detect_leaks=1:verbosity=1")
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
            let (code, out, err) = self.lsan_run(id, &format!("{tag}_lsan_rc"), src, files, *opt, RuntimeVariant::Rc);
            assert!(err.contains("AddressSanitizer"), "{id} at {tag}: asan is not armed");
            assert!(!err.contains("ERROR: AddressSanitizer"), "{id} at {tag}: asan reports\n{err}");
            assert!(!err.contains("leaked in"), "{id} at {tag}: lsan reports a leak\n{err}");
            assert_eq!(code, 0, "{id} at {tag}: exit\n{err}");
            assert_eq!(out, stdout, "{id} at {tag}");
            let (_, _, err) = self.lsan_run(id, &format!("{tag}_lsan_leak"), src, files, *opt, RuntimeVariant::Leak);
            assert!(err.contains("leaked in"), "{id} at {tag}: the leak control must make lsan fire\n{err}");
        }
    }
}

const RC1_MODULE: &str = r#"pub struct W { pub r: Rc<i64> }

pub fn get(w: W, y: Rc<i64>) -> i64 {
    return Rc::get(w.r) * 1000 + Rc::get(y)
}

pub enum E { A(string), B }
"#;

const RC1_IMPORTED_CARRIER_LITERAL: &str = r#"needs m

fn mk(i: i64) -> m.W {
    let a: Rc<i64> = Rc::new(i)
    return m.W { r: a }
}

fn main() -> i64 {
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < 3 {
        let w: m.W = mk(i)
        let p: Rc<i64> = Rc::new(9)
        n = n + m.get(w, p)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn rc_1_an_imported_carrier_literal_takes_its_share() {
    let h = Harness::new();
    let files = [("m.aelys", RC1_MODULE)];
    h.differential_row("rc-1", RC1_IMPORTED_CARRIER_LITERAL, &files, "3027\n", (6, 6));
    h.lsan_row("rc-1", RC1_IMPORTED_CARRIER_LITERAL, &files, "3027\n");
    h.assert_legs(24);
}

const RC2_IMPORTED_ENUM_OWNS_ITS_STRING: &str = r#"needs m

fn victim(n: i64) -> string { return "ab" + "{n}" }

fn main() -> i64 {
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < 10 {
        let e: m.E = m.E::A(victim(i))
        match e {
            m.E::A(s) => { n = n + s.len }
            m.E::B => { n = n + 1 }
        }
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn rc_2_an_imported_enum_owns_its_string() {
    let h = Harness::new();
    let files = [("m.aelys", RC1_MODULE)];
    h.differential_row("rc-2", RC2_IMPORTED_ENUM_OWNS_ITS_STRING, &files, "30\n", (20, 20));
    h.lsan_row("rc-2", RC2_IMPORTED_ENUM_OWNS_ITS_STRING, &files, "30\n");
    h.assert_legs(24);
}

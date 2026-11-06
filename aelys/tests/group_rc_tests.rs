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
    ASAN_DIR
        .get_or_init(|| {
            let dir =
                std::env::temp_dir().join(format!("aelys-group-rc-asan-{}", std::process::id()));
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
                        .expect("clang is required for the lsan rows");
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
        .unwrap_or_else(|e| panic!("{id} at {leg} must compile:\n{e}"));
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
                "{id} at {tag}: the leak control must make lsan fire\n{err}"
            );
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
    h.differential_row(
        "rc-1",
        RC1_IMPORTED_CARRIER_LITERAL,
        &files,
        "3027\n",
        (6, 6),
    );
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
    h.differential_row(
        "rc-2",
        RC2_IMPORTED_ENUM_OWNS_ITS_STRING,
        &files,
        "30\n",
        (20, 20),
    );
    h.lsan_row("rc-2", RC2_IMPORTED_ENUM_OWNS_ITS_STRING, &files, "30\n");
    h.assert_legs(24);
}

const RC3_Q10: &str = r#"
fn main() -> i64 {
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < 3 {
        let base: [i64; 2] = [0, 0]
        let mut s: &[i64] = base[0..2]
        if i >= 0 {
            let a: Rc<[i64; 4]> = Rc::new([i, 5, 6, 7])
            s = Rc::get(a)[0..2]
        }
        let p: Rc<[i64; 4]> = Rc::new([9, 9, 9, 9])
        n = n + s[0] * 1000 + Rc::get(p)[0]
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn rc_3_q10_a_slice_into_a_dying_rc_payload_keeps_the_payload_alive() {
    let h = Harness::new();
    h.differential_row("rc-3", RC3_Q10, &[], "3027\n", (6, 6));
    h.lsan_row("rc-3", RC3_Q10, &[], "3027\n");
    h.assert_legs(24);
}

const RC4_LOOP_ASSIGN: &str = r#"
fn main() -> i64 {
    let base: [i64; 2] = [0, 0]
    let mut s: &[i64] = base[0..2]
    let mut i: i64 = 0
    while i < 3 {
        let a: Rc<[i64; 2]> = Rc::new([i, 5])
        s = Rc::get(a)[0..2]
        i = i + 1
    }
    let p: Rc<[i64; 2]> = Rc::new([9, 9])
    println(s[0] * 1000 + Rc::get(p)[0])
    return 0
}
"#;

#[test]
fn rc_4_a_view_assigned_in_a_loop_holds_the_payload_of_its_turn() {
    let h = Harness::new();
    h.differential_row("rc-4", RC4_LOOP_ASSIGN, &[], "2009\n", (4, 4));
    h.lsan_row("rc-4", RC4_LOOP_ASSIGN, &[], "2009\n");
    h.assert_legs(24);
}

const RC5_VIEW_COPY: &str = r#"
fn main() -> i64 {
    let base: [i64; 2] = [0, 0]
    let mut s: &[i64] = base[0..2]
    let c: bool = true
    if c {
        let a: Rc<[i64; 2]> = Rc::new([1, 5])
        let t: &[i64] = Rc::get(a)[0..2]
        s = t
    }
    let p: Rc<[i64; 2]> = Rc::new([9, 9])
    println(s[0] * 1000 + Rc::get(p)[0])
    return 0
}
"#;

#[test]
fn rc_5_a_view_copied_from_another_view_binding_takes_its_own_share() {
    let h = Harness::new();
    h.differential_row("rc-5", RC5_VIEW_COPY, &[], "1009\n", (2, 2));
    h.assert_legs(18);
}

const RC6_REF_ELEM: &str = r#"
fn main() -> i64 {
    let base: [i64; 2] = [0, 0]
    let mut s: &i64 = &base[0]
    {
        let a: Rc<[i64; 2]> = Rc::new([1, 5])
        s = &Rc::get(a)[0]
    }
    let p: Rc<[i64; 2]> = Rc::new([9, 9])
    println(*s * 1000 + Rc::get(p)[0])
    return 0
}
"#;

#[test]
fn rc_6_a_reference_into_an_rc_payload_keeps_the_payload_alive() {
    let h = Harness::new();
    h.differential_row("rc-6", RC6_REF_ELEM, &[], "1009\n", (2, 2));
    h.assert_legs(18);
}

const RC7_CARRIER_FIELD: &str = r#"
struct H { r: Rc<[i64; 2]> }
fn main() -> i64 {
    let base: [i64; 2] = [0, 0]
    let mut s: &[i64] = base[0..2]
    {
        let h: H = H { r: Rc::new([1, 5]) }
        s = Rc::get(h.r)[0..2]
    }
    let p: Rc<[i64; 2]> = Rc::new([9, 9])
    println(s[0] * 1000 + Rc::get(p)[0])
    return 0
}
"#;

#[test]
fn rc_7_a_view_through_a_carrier_field_keeps_the_payload_alive() {
    let h = Harness::new();
    h.differential_row("rc-7", RC7_CARRIER_FIELD, &[], "1009\n", (2, 2));
    h.assert_legs(18);
}

const RC8_BACKEDGE: &str = r#"
fn main() -> i64 {
    let base: [i64; 2] = [0, 0]
    let mut s: &[i64] = base[0..2]
    let mut t: &[i64] = base[0..2]
    let mut i: i64 = 0
    while i < 2 {
        s = t
        {
            let a: Rc<[i64; 2]> = Rc::new([i + 1, 5])
            t = Rc::get(a)[0..2]
        }
        i = i + 1
    }
    let p: Rc<[i64; 2]> = Rc::new([9, 9])
    println(s[0] * 1000 + Rc::get(p)[0])
    return 0
}
"#;

#[test]
fn rc_8_a_view_copied_before_its_source_is_lowered_survives_the_back_edge() {
    let h = Harness::new();
    h.differential_row("rc-8", RC8_BACKEDGE, &[], "1009\n", (3, 3));
    h.assert_legs(18);
}

const RC9_ARG_HOLDER: &str = r#"
fn f(s: &[i64], z: i64) -> i64 {
    let p: Rc<[i64; 2]> = Rc::new([9, 9])
    return s[0] * 1000 + Rc::get(p)[0]
}
fn main() -> i64 {
    let base: [i64; 2] = [0, 0]
    let mut t: &[i64] = base[0..2]
    {
        let a: Rc<[i64; 2]> = Rc::new([1, 5])
        t = Rc::get(a)[0..2]
    }
    println(f(t, { t = base[0..2]; 0 }))
    return 0
}
"#;

#[test]
fn rc_9_a_call_argument_holds_its_view_while_a_sibling_rebinds_the_source() {
    let h = Harness::new();
    h.differential_row("rc-9", RC9_ARG_HOLDER, &[], "1009\n", (2, 2));
    h.lsan_row("rc-9", RC9_ARG_HOLDER, &[], "1009\n");
    h.assert_legs(24);
}

const RC10_FOR_HOLDER: &str = r#"
fn main() -> i64 {
    let base: [i64; 2] = [0, 0]
    let mut t: &[i64] = base[0..2]
    {
        let a: Rc<[i64; 2]> = Rc::new([1, 5])
        t = Rc::get(a)[0..2]
    }
    let mut n: i64 = 0
    for x in t {
        t = base[0..2]
        let p: Rc<[i64; 2]> = Rc::new([9, 9])
        n = n * 100 + x * 10 + Rc::get(p)[0]
    }
    println(n)
    return 0
}
"#;

#[test]
fn rc_10_a_for_loop_holds_its_collection_while_the_body_rebinds_it() {
    let h = Harness::new();
    h.differential_row("rc-10", RC10_FOR_HOLDER, &[], "1959\n", (3, 3));
    h.lsan_row("rc-10", RC10_FOR_HOLDER, &[], "1959\n");
    h.assert_legs(24);
}

const RC11_RETURN_IN_ARG: &str = r#"
fn keep(x: &[i64], k: i64) -> &[i64] { return x }
fn f(c: bool) -> i64 {
    let base: [i64; 2] = [0, 0]
    let mut s: &[i64] = base[0..2]
    {
        let a: Rc<[i64; 2]> = Rc::new([1, 5])
        s = keep(Rc::get(a)[0..2], {
            if c { return 7 }
            1
        })
    }
    let p: Rc<[i64; 2]> = Rc::new([9, 9])
    return s[0] * 1000 + Rc::get(p)[0]
}
fn main() -> i64 {
    println(f(false))
    println(f(true))
    return 0
}
"#;

#[test]
fn rc_11_a_return_inside_a_view_argument_releases_the_captured_share() {
    let h = Harness::new();
    h.differential_row("rc-11", RC11_RETURN_IN_ARG, &[], "1009\n7\n", (3, 3));
    h.assert_legs(18);
}

const RC12_WHILE_COND: &str = r#"
fn first(x: &[i64]) -> i64 { return x[0] }
fn main() -> i64 {
    let a: Rc<[i64; 2]> = Rc::new([1, 5])
    let mut i: i64 = 0
    while i < first(Rc::get(a)[0..2]) * 3 {
        i = i + 1
    }
    println(i)
    return 0
}
"#;

#[test]
fn rc_12_a_view_argument_in_a_loop_condition_is_released_each_turn() {
    let h = Harness::new();
    h.differential_row("rc-12", RC12_WHILE_COND, &[], "3\n", (1, 1));
    h.assert_legs(18);
}

const RC13_SELF_PICK: &str = r#"
fn pick<T>(c: bool, x: T, y: T) -> T { if c { return x } return y }
fn main() -> i64 {
    let base: [i64; 2] = [3, 4]
    let a: Rc<[i64; 2]> = Rc::new([1, 5])
    let mut s: &[i64] = base[0..2]
    s = pick(true, Rc::get(a)[0..2], s)
    println(s[0])
    return 0
}
"#;

#[test]
fn rc_13_a_view_that_may_point_into_itself_and_a_new_rc_is_refused() {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, RC13_SELF_PICK).expect("write");
    let err =
        compile_file_with_llvm_variant(&root, OptimizationLevel::None, false, RuntimeVariant::Rc)
            .expect_err("rc-13 must be refused");
    let text = err.to_string();
    assert!(
        text.contains("E0902") && text.contains("[rc-view]"),
        "rc-13: {text}"
    );
}

const RC14_IF_EXPR: &str = r#"
fn main() -> i64 {
    let base: [i64; 2] = [0, 0]
    let mut s: &[i64] = base[0..2]
    let c: bool = true
    {
        let a: Rc<[i64; 2]> = Rc::new([1, 5])
        s = if c { Rc::get(a)[0..2] } else { base[0..2] }
    }
    let p: Rc<[i64; 2]> = Rc::new([9, 9])
    println(s[0] * 1000 + Rc::get(p)[0])
    return 0
}
"#;

#[test]
fn rc_14_a_view_chosen_by_an_if_expression_keeps_its_payload_alive() {
    let h = Harness::new();
    h.differential_row("rc-14", RC14_IF_EXPR, &[], "1009\n", (2, 2));
    h.assert_legs(18);
}

const RC15_MATCH_EXPR: &str = r#"
enum K {
    A,
    B,
}

fn main() -> i64 {
    let base: [i64; 2] = [0, 0]
    let mut s: &[i64] = base[0..2]
    let k: K = K::A
    {
        let a: Rc<[i64; 2]> = Rc::new([1, 5])
        s = match k {
            K::A => Rc::get(a)[0..2],
            K::B => base[0..2],
        }
    }
    let p: Rc<[i64; 2]> = Rc::new([9, 9])
    println(s[0] * 1000 + Rc::get(p)[0])
    return 0
}
"#;

#[test]
fn rc_15_a_view_chosen_by_a_match_expression_keeps_its_payload_alive() {
    let h = Harness::new();
    h.differential_row("rc-15", RC15_MATCH_EXPR, &[], "1009\n", (2, 2));
    h.assert_legs(18);
}

const RC16_REF_FROM_VIEW: &str = r#"
fn first<T>(x: &[T]) -> &T {
    return &x[0]
}

fn main() -> i64 {
    let z: i64 = 0
    let mut r: &i64 = &z
    {
        let a: Rc<[i64; 2]> = Rc::new([1, 5])
        let s: &[i64] = Rc::get(a)[0..2]
        r = first(s)
    }
    let p: Rc<[i64; 2]> = Rc::new([9, 9])
    println(*r * 1000 + Rc::get(p)[0])
    return 0
}
"#;

#[test]
fn rc_16_a_reference_returned_from_a_view_argument_keeps_the_payload_alive() {
    let h = Harness::new();
    h.differential_row("rc-16", RC16_REF_FROM_VIEW, &[], "1009\n", (2, 2));
    h.assert_legs(18);
}

const RC17_CONTINUE_IN_ARG: &str = r#"
fn sum2(x: &[i64], y: &[i64]) -> i64 {
    return x[0] + y[0]
}

fn main() -> i64 {
    let base: [i64; 2] = [0, 0]
    let a: Rc<[i64; 2]> = Rc::new([1, 5])
    let mut acc: i64 = 0
    for i in 0..4 {
        acc = acc + sum2(Rc::get(a)[0..2], {
            if i >= 2 {
                continue
            }
            base[0..2]
        })
    }
    println(acc)
    return 0
}
"#;

#[test]
fn rc_17_a_continue_inside_a_held_argument_releases_the_held_share() {
    let h = Harness::new();
    h.differential_row("rc-17", RC17_CONTINUE_IN_ARG, &[], "2\n", (1, 1));
    h.lsan_row("rc-17", RC17_CONTINUE_IN_ARG, &[], "2\n");
    h.assert_legs(24);
}

const RC18_BREAK_IN_RHS: &str = r#"
fn pick(c: bool, x: &[i64], y: &[i64]) -> &[i64] {
    if c {
        return x
    }
    return y
}

fn main() -> i64 {
    let base: [i64; 2] = [0, 0]
    let a: Rc<[i64; 2]> = Rc::new([1, 5])
    let mut acc: i64 = 0
    let mut i: i64 = 0
    while i < 4 {
        i = i + 1
        let t: &[i64] = pick(true, Rc::get(a)[0..2], {
            if i >= 2 {
                break
            }
            base[0..2]
        })
        acc = acc + t[0]
    }
    println(acc)
    return 0
}
"#;

#[test]
fn rc_18_a_break_inside_a_view_right_hand_side_releases_the_captured_share() {
    let h = Harness::new();
    h.differential_row("rc-18", RC18_BREAK_IN_RHS, &[], "1\n", (1, 1));
    h.assert_legs(18);
}

const RC19_BORROWED_TAIL: &str = r#"
struct W { r: Rc<i64> }

fn get(x: Rc<i64>) -> i64 {
    return Rc::get(x)
}

fn from_param(p: W) -> i64 {
    let w: W = {
        let k: i64 = 1
        p
    }
    return Rc::get(w.r)
}

fn rc_param(x: Rc<i64>, c: bool) -> i64 {
    return get(if c { x } else { x })
}

fn main() -> i64 {
    let o: W = W { r: Rc::new(5) }
    println(from_param(o))
    println(rc_param(o.r, true))
    return 0
}
"#;

#[test]
fn rc_19_a_carrier_bound_from_a_borrowed_block_tail_takes_its_share() {
    let h = Harness::new();
    h.differential_row("rc-19", RC19_BORROWED_TAIL, &[], "5\n5\n", (1, 1));
    h.assert_legs(18);
}

const RC20_GENERIC_CARRIER: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn churn() -> Vec<string> {
    let mut keep: Vec<string> = Vec::new()
    let mut i: i64 = 10
    while i < 50 {
        Vec::push(keep, "Q" + "{i}")
        i = i + 1
    }
    return keep
}

struct B { r: Rc<i64> }
fn keep<T>(x: T) -> T { return x }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(41)
    let b: B = B { r: a }
    let c: B = keep(b)
    let k = churn()
    println("{Rc::get(c.r) + 1}")
    return 0
}
"#;

#[test]
fn rc_20_a_carrier_through_a_generic_identity_keeps_its_share() {
    let h = Harness::new();
    h.differential_row("rc-20", RC20_GENERIC_CARRIER, &[], "42\n", (82, 82));
    h.assert_legs(18);
}

const RC21_LET_FROM_CALL: &str = r#"
fn mk(v: i64) -> Rc<i64> {
    return Rc::new(v)
}

fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 3 {
        let a: Rc<i64> = mk(i + 4)
        n = n + Rc::get(a)
        i = i + 1
    }
    let b: Rc<i64> = mk(1)
    println(n + Rc::get(b))
    return 0
}
"#;

#[test]
fn rc_21_an_rc_binding_initialized_by_a_call_claims_the_returned_share() {
    let h = Harness::new();
    h.differential_row("rc-21", RC21_LET_FROM_CALL, &[], "16\n", (4, 4));
    h.assert_legs(18);
}

const RC22_MATCH_ARM_BORROW: &str = r#"
struct W { r: Rc<i64> }
enum H { Cell(Rc<i64>) }

fn mkh(v: i64) -> H {
    return H::Cell(Rc::new(v))
}

fn f(q: W, v: i64) -> i64 {
    let w: W = match mkh(v) {
        H::Cell(r) => q,
    }
    return Rc::get(w.r)
}

fn main() -> i64 {
    let o: W = W { r: Rc::new(7) }
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 3 {
        n = n + f(o, i)
        i = i + 1
    }
    let p: Rc<i64> = Rc::new(9)
    println(n * 100 + Rc::get(o.r) + Rc::get(p))
    return 0
}
"#;

#[test]
fn rc_22_a_match_arm_handing_back_a_borrowed_carrier_shares_it() {
    let h = Harness::new();
    h.differential_row("rc-22", RC22_MATCH_ARM_BORROW, &[], "2116\n", (5, 5));
    h.assert_legs(18);
}

const RC23_TWO_DISCIPLINES: &str = r#"
struct M { s: string, r: Rc<i64> }

fn rm(m: M) -> M {
    return m
}

fn f(m: M) -> i64 {
    let k: M = rm(m)
    return Rc::get(k.r) + k.s.len
}

fn main() -> i64 {
    let mut t: string = "x"
    let mut j: i64 = 0
    while j < 3 {
        t = t + "{j}"
        j = j + 1
    }
    let om: M = M { s: t, r: Rc::new(1) }
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 3 {
        n = n + f(om)
        i = i + 1
    }
    let p: Rc<i64> = Rc::new(9)
    println(n * 100 + Rc::get(om.r) + om.s.len + Rc::get(p))
    return 0
}
"#;

#[test]
fn rc_23_a_returned_two_discipline_carrier_shares_its_string_and_its_rc() {
    let h = Harness::new();
    h.differential_row("rc-23", RC23_TWO_DISCIPLINES, &[], "1514\n", (8, 8));
    h.lsan_row("rc-23", RC23_TWO_DISCIPLINES, &[], "1514\n");
    h.assert_legs(24);
}

const RC24_FRESH_ARG: &str = r#"
fn use_it(x: Rc<i64>) -> i64 {
    return Rc::get(x)
}

fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 3 {
        n = n + use_it(Rc::new(i + 3))
        i = i + 1
    }
    let p: Rc<i64> = Rc::new(9)
    println(n * 100 + Rc::get(p))
    return 0
}
"#;

#[test]
fn rc_24_a_fresh_rc_passed_as_an_argument_is_released_after_the_call() {
    let h = Harness::new();
    h.differential_row("rc-24", RC24_FRESH_ARG, &[], "1209\n", (4, 4));
    h.lsan_row("rc-24", RC24_FRESH_ARG, &[], "1209\n");
    h.assert_legs(24);
}

const RC25_CLOSURE_ENV: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 5 {
        let s: string = victim(i)
        let f = fn () -> i64 { return s.len }
        n = n + f()
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn rc_25_a_closure_environment_is_released() {
    let h = Harness::new();
    h.differential_row("rc-25", RC25_CLOSURE_ENV, &[], "15\n", (15, 15));
    h.lsan_row("rc-25", RC25_CLOSURE_ENV, &[], "15\n");
    h.assert_legs(24);
}

const RC26_MODULE: &str = r#"pub fn keep<T>(x: T) -> T {
    return x
}
"#;

const RC26_IMPORTED_GENERIC_LENDS: &str = r#"needs m

fn use_it(x: Rc<i64>) -> i64 {
    return Rc::get(x)
}

fn main() -> i64 {
    let o: Rc<i64> = Rc::new(41)
    println(use_it(m.keep(o)))
    let a: Rc<i64> = m.keep(o)
    println(use_it(a))
    println(Rc::get(o))
    return 0
}
"#;

#[test]
fn rc_26_an_imported_generic_returning_its_parameter_lends_an_rc() {
    let h = Harness::new();
    let files = [("m.aelys", RC26_MODULE)];
    h.differential_row(
        "rc-26",
        RC26_IMPORTED_GENERIC_LENDS,
        &files,
        "41\n41\n41\n",
        (1, 1),
    );
    h.assert_legs(18);
}

const RC27: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn zero() -> i64 { return 0 }
struct W { f: fn() -> i64 }
fn clobber(h: Rc<W>, f: fn() -> i64) -> i64 {
    h.f = zero
    let t: string = victim(88888)
    return f() + t.len
}
fn main() -> i64 {
    let s: string = victim(12345)
    let a = Rc::new(W { f: fn () -> i64 { return s.len + s.bytes[0] } })
    let n = clobber(a, Rc::get(a).f)
    println("{n}")
    return 0
}
"#;

#[test]
fn rc_27_a_store_through_an_rc_handle_keeps_a_closure_field_alive_for_a_running_call() {
    let h = Harness::new();
    h.differential_row("rc-27", RC27, &[], "111\n", (6, 4));
    h.assert_legs(18);
}

const RC28: &str = r#"
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
    let n = clobber(a, Rc::get(a).s)
    println("{n}")
    return 0
}
"#;

#[test]
fn rc_28_a_store_through_an_rc_handle_keeps_a_string_argument_alive() {
    let h = Harness::new();
    h.differential_row("rc-28", RC28, &[], "111\n", (7, 6));
    h.assert_legs(18);
}

const RC29: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct W { s: string }
fn main() -> i64 {
    let a = Rc::new(W { s: victim(12345) })
    let b = a
    let bs = Rc::get(a).s.bytes
    b.s = victim(1)
    let t: string = other(88888)
    println("{bs[0]}|{bs.len}|{t.len}")
    return 0
}
"#;

#[test]
fn rc_29_a_store_through_an_rc_handle_keeps_a_bytes_view_of_the_field_alive() {
    let h = Harness::new();
    h.differential_row("rc-29", RC29, &[], "97|7|7\n", (14, 13));
    h.assert_legs(18);
}

const RC30: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn other(n: i64) -> string { return "ZZ" + "{n}" }
struct In { s: string, k: i64 }
struct W { i: In }
fn clobber(h: Rc<W>, x: In) -> i64 {
    h.i = In { s: victim(1), k: 0 }
    let t: string = other(88888)
    return x.s.bytes[0] + x.s.len + t.len + x.k
}
fn main() -> i64 {
    let a = Rc::new(W { i: In { s: victim(12345), k: 5 } })
    let n = clobber(a, Rc::get(a).i)
    println("{n}")
    return 0
}
"#;

#[test]
fn rc_30_a_store_through_an_rc_handle_keeps_a_carrier_argument_alive() {
    let h = Harness::new();
    h.differential_row("rc-30", RC30, &[], "116\n", (7, 6));
    h.assert_legs(18);
}

const RC31: &str = r#"
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
    let n = clobber(a, Rc::get(a).a[1])
    println("{n}")
    return 0
}
"#;

#[test]
fn rc_31_a_store_through_an_rc_handle_keeps_an_element_argument_alive() {
    let h = Harness::new();
    h.differential_row("rc-31", RC31, &[], "111\n", (9, 8));
    h.assert_legs(18);
}

fn refused(id: &str, src: &str, code: &str) {
    let dir = tempdir().expect("tempdir");
    let root = dir.path().join("root.aelys");
    fs::write(&root, src).expect("write");
    let err =
        compile_file_with_llvm_variant(&root, OptimizationLevel::None, false, RuntimeVariant::Rc)
            .expect_err(id);
    let text = err.to_string();
    assert!(text.contains(code), "{id}: {text}");
}

#[test]
fn rc_32_an_array_of_rc_is_refused() {
    refused(
        "rc-32",
        "fn main() -> i64 {\n    let a: [Rc<i64>; 2] = [Rc::new(1), Rc::new(2)]\n    return Rc::get(a[0])\n}\n",
        "E0410",
    );
}

#[test]
fn rc_33_a_vec_as_a_type_argument_is_refused() {
    refused(
        "rc-33",
        "fn keep<T>(x: T) -> T { return x }\nfn main() -> i64 {\n    let v: Vec<i64> = Vec::new()\n    let w: Vec<i64> = keep(v)\n    return Vec::len(w)\n}\n",
        "E0412",
    );
}

const RC34_GENERIC_FRESH_RC: &str = r#"
fn use_it(x: Rc<i64>) -> i64 {
    return Rc::get(x)
}
fn mk() -> Rc<i64> {
    return Rc::new(7)
}
fn app<T>(f: fn() -> T) -> T {
    return f()
}
fn main() -> i64 {
    let a: Rc<i64> = app(mk)
    println(use_it(app(mk)) + Rc::get(a))
    return 0
}
"#;

#[test]
fn rc_34_a_fresh_rc_returned_as_a_bare_type_parameter_leaks() {
    let h = Harness::new();
    h.differential_row("rc-34", RC34_GENERIC_FRESH_RC, &[], "14\n", (2, 0));
    h.assert_legs(18);
}

const RC35_LONG_LIST: &str = r#"
struct N { v: i64, next: Rc<N> }
struct H { r: Rc<N> }
fn main() -> i64 {
    let mut h = H { r: Rc::null() }
    let mut i: i64 = 0
    while i < 1000000 {
        h = H { r: Rc::new(N { v: i, next: h.r }) }
        i = i + 1
    }
    println("{Rc::get(h.r).v}")
    return 0
}
"#;

#[test]
fn rc_35_a_long_list_is_released_in_linear_time_under_the_cycle_collector() {
    let h = Harness::new();
    let root = h.stage("rc-35", "o2_cycles", RC35_LONG_LIST, &[]);
    let exe = h
        .compile_at(
            "rc-35",
            "o2_cycles",
            &root,
            OptimizationLevel::Standard,
            RuntimeVariant::RcCycles,
        )
        .expect("rc-35 must link");
    let mut child = Command::new(&exe)
        .env("AELYS_RC_STATS", "1")
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("run rc-35");
    let start = std::time::Instant::now();
    while child.try_wait().expect("wait rc-35").is_none() {
        if start.elapsed() > std::time::Duration::from_secs(30) {
            let _ = child.kill();
            panic!("rc-35: releasing a list of 10^6 nodes took more than 30 s");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = child.wait_with_output().expect("rc-35 output");
    assert_eq!(String::from_utf8_lossy(&out.stdout), "999999\n");
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("[rc] allocs=1000000 frees=1000000"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

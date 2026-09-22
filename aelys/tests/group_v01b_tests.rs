// the tracked block of run 0.1 take two: every row here answers differently at the run's anchor

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
    fn stage(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let dir = self.dir.path().join(slug(id, tag));
        fs::create_dir_all(&dir).expect("stage dir");
        copy_tree(&self.library, &dir.join("std"));
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

    // the compiler's own advice must not hand back the escaping view s[i] was removed for

    // leak never frees, so its answer is right by construction and any divergence is an early free
    fn differential_row(&self, id: &str, src: &str, stdout: &str, counted: (i64, i64)) {
        self.differential_row_by_level(id, src, stdout, &|_| counted);
    }

    fn differential_row_by_level(
        &self,
        id: &str,
        src: &str,
        stdout: &str,
        counted_at: &dyn Fn(&str) -> (i64, i64),
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
                    let root = self.stage(id, &leg, src);
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
                let counted = counted_at(tag);
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

const V1_A_BOUND_STRING_IS_FREED: &str = r#"
fn main() -> i64 {
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < 20 {
        let s: string = "row " + "{i}"
        n = n + s.len
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn v01b_1_a_bound_string_is_freed_when_its_binding_dies() {
    let h = Harness::new();
    h.differential_row("v01b-1", V1_A_BOUND_STRING_IS_FREED, "110\n", (40, 40));
    h.assert_legs(18);
}

const V2_A_VEC_OWNS_ITS_ELEMENTS: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 10 {
        let mut v: Vec<string> = Vec::new()
        Vec::push(v, victim(i))
        Vec::push(v, victim(i + 1))
        n = n + v[0].len + v[1].len
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn v01b_2_a_vec_of_strings_owns_its_elements() {
    let h = Harness::new();
    h.differential_row("v01b-2", V2_A_VEC_OWNS_ITS_ELEMENTS, "61\n", (50, 50));
    h.assert_legs(18);
}

const V3_A_STRUCT_OWNS_ITS_STRINGS: &str = r#"
struct P { a: string, b: string }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 10 {
        let p: P = P { a: victim(i), b: victim(i + 1) }
        let q: P = p
        n = n + q.a.len + q.b.len
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn v01b_3_a_struct_owns_the_strings_it_holds() {
    let h = Harness::new();
    h.differential_row("v01b-3", V3_A_STRUCT_OWNS_ITS_STRINGS, "61\n", (40, 40));
    h.assert_legs(18);
}

const V4_AN_ENUM_OWNS_ITS_CHARGE: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 10 {
        let o: Option<string> = Option::Some(victim(i))
        match o {
            Option::Some(x) => { n = n + x.len }
            Option::None => { n = n + 1 }
        }
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn v01b_4_an_enum_owns_the_charge_its_tag_names() {
    let h = Harness::new();
    h.differential_row("v01b-4", V4_AN_ENUM_OWNS_ITS_CHARGE, "30\n", (20, 20));
    h.assert_legs(18);
}

const V5_A_GLOBAL_GIVES_BACK: &str = r#"
let mut g: string = "g0"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 10 {
        g = victim(i)
        n = n + g.len
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn v01b_5_a_reassigned_global_gives_back_what_it_held() {
    let h = Harness::new();
    h.differential_row("v01b-5", V5_A_GLOBAL_GIVES_BACK, "30\n", (20, 19));
    h.assert_legs(18);
}

const V6_A_VEC_OF_ENUMS_STRIDES: &str = r#"
fn main() -> i64 {
    let mut v: Vec<Option<i64>> = Vec::new()
    Vec::push(v, Option::Some(11))
    Vec::push(v, Option::Some(22))
    Vec::push(v, Option::Some(33))
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 3 {
        match v[i] {
            Option::Some(x) => { n = n * 100 + x }
            Option::None => { n = n * 100 }
        }
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn v01b_6_a_vec_of_enums_indexes_at_its_stride() {
    let h = Harness::new();
    h.differential_row("v01b-6", V6_A_VEC_OF_ENUMS_STRIDES, "112233\n", (1, 1));
    h.assert_legs(18);
}

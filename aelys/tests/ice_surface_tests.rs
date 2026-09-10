use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Once;
use tempfile::{TempDir, tempdir};

mod common;
use common::backend_family_code;

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
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

fn named_code(rendered: &str) -> Option<String> {
    let at = rendered.find("error[E")?;
    let rest = &rendered[at + 6..];
    let end = rest.find(']')?;
    Some(rest[..end].to_string())
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

    fn compile_at(
        &self,
        id: &str,
        tag: &str,
        root: &Path,
        opt: OptimizationLevel,
    ) -> Option<PathBuf> {
        match compile_file_with_llvm_variant(root, opt, false, RuntimeVariant::Rc) {
            Ok(()) => {}
            Err(err) => {
                if linker_unavailable(&err.to_string()) {
                    self.linker_skips.set(self.linker_skips.get() + 1);
                    return None;
                }
                panic!("{id} at {tag} must compile:\n{}", err);
            }
        }
        let exe = exe_path_for(root);
        exe.is_file().then_some(exe)
    }

    fn assert_row(&self, id: &str, exe: &Path, tag: &str, stdout: &str, allocs: i64, frees: i64) {
        for (alloc_name, alloc) in ALLOCATORS {
            let o = self.run(exe, *alloc);
            self.legs.set(self.legs.get() + 1);
            assert_eq!(
                o.exit, 0,
                "{id} at {tag}/{alloc_name} must exit 0\nstderr:\n{}",
                o.stderr
            );
            assert_eq!(
                o.stdout, stdout,
                "{id} at {tag}/{alloc_name}: stdout MUST be {stdout:?}"
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
                 allocs={a} frees={f}"
            );
        }
    }

    fn value_row(&self, id: &str, src: &str, stdout: &str, allocs: i64, frees: i64) {
        for (tag, opt) in LEVELS {
            let path = self.write(id, tag, src);
            let Some(exe) = self.compile_at(id, tag, &path, *opt) else {
                assert!(
                    linker_skip_declared(),
                    "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped value \
                     row carries no runtime evidence at all"
                );
                return;
            };
            self.assert_row(id, &exe, tag, stdout, allocs, frees);
        }
    }

    fn twin_row(&self, id: &str, left: &str, right: &str) {
        for (tag, opt) in LEVELS {
            let lp = self.write(&format!("{id}-l"), tag, left);
            let rp = self.write(&format!("{id}-r"), tag, right);
            let (Some(lx), Some(rx)) = (
                self.compile_at(id, tag, &lp, *opt),
                self.compile_at(id, tag, &rp, *opt),
            ) else {
                assert!(
                    linker_skip_declared(),
                    "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped twin \
                     row carries no runtime evidence at all"
                );
                return;
            };
            for (alloc_name, alloc) in ALLOCATORS {
                let lo = self.run(&lx, *alloc);
                let ro = self.run(&rx, *alloc);
                self.legs.set(self.legs.get() + 1);
                assert_eq!(lo.exit, 0, "{id} at {tag}/{alloc_name}: left must exit 0");
                assert_eq!(ro.exit, 0, "{id} at {tag}/{alloc_name}: right must exit 0");
                assert_eq!(
                    lo.stdout, ro.stdout,
                    "{id} at {tag}/{alloc_name}: the two spellings MUST print the same"
                );
                assert_eq!(
                    lo.stats, ro.stats,
                    "{id} at {tag}/{alloc_name}: the two spellings MUST account the same"
                );
                assert!(
                    lo.stats.is_some(),
                    "{id} at {tag}/{alloc_name}: no [rc] stats line, the comparison is vacuous"
                );
            }
        }
    }

    fn fenced_row(&self, id: &str, src: &str, code: &str, says: &str) {
        for (tag, opt) in LEVELS {
            let path = self.write(id, tag, src);
            let rendered = match lower_file_to_air(&path, *opt) {
                Ok(_) => panic!("{id} at {tag} MUST be rejected, but it was accepted:\n{src}"),
                Err(rendered) => rendered,
            };
            self.legs.set(self.legs.get() + 1);
            assert!(
                rendered.contains(&format!("[{code}]")) && rendered.contains(says),
                "{id} at {tag} MUST be refused by {code} saying {says:?}\ngot:\n{rendered}"
            );
        }
    }

    fn no_ice_row(&self, id: &str, src: &str) {
        for (tag, opt) in LEVELS {
            let path = self.write(id, tag, src);
            self.legs.set(self.legs.get() + 1);
            match lower_file_to_air(&path, *opt) {
                Ok(_) => {}
                Err(rendered) => {
                    let leaked = backend_family_code(&rendered);
                    assert!(
                        leaked.is_none(),
                        "{id} at {tag}: a builtin intercept reported a backend failure \
                         ({leaked:?}) to a user program\n{rendered}"
                    );
                    assert!(
                        named_code(&rendered).is_some(),
                        "{id} at {tag}: rejected with no named code at all\n{rendered}"
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
             is the confident zero this run keeps hitting"
        );
    }
}

const REPAIR: &str = "bind it to a name first and use that binding";

const I1_LEN_OF_A_TEMPORARY: &str = r#"
fn mkv() -> Vec<i64> { return vec[1, 2, 3, 4, 5] }
fn main() -> i64 {
    println(Vec::len(mkv()))
    println(mkv().len)
    println(Vec::len(vec[7, 8]))
    return 0
}
"#;

#[test]
fn ice1_a_length_of_a_temporary_answers_the_length() {
    let h = Harness::new();
    // only the literal temporary is released; the two returned by mkv() still leak
    h.value_row("ICE-1", I1_LEN_OF_A_TEMPORARY, "5\n5\n2\n", 3, 1);
    h.assert_legs(6);
}

const I2_CALL_SPELLING: &str = r#"
fn mkv() -> Vec<i64> { return vec[1, 2, 3, 4, 5] }
fn main() -> i64 {
    println(Vec::len(mkv()))
    return 0
}
"#;

const I2_MEMBER_SPELLING: &str = r#"
fn mkv() -> Vec<i64> { return vec[1, 2, 3, 4, 5] }
fn main() -> i64 {
    println(mkv().len)
    return 0
}
"#;

#[test]
fn ice2_the_call_spelling_is_the_member_spelling() {
    let h = Harness::new();
    h.twin_row("ICE-2", I2_CALL_SPELLING, I2_MEMBER_SPELLING);
    h.assert_legs(6);
}

const I3_AS_SLICE_OF_A_TEMPORARY: &str = r#"
fn mkv() -> Vec<i64> { return vec[1, 2, 3] }
fn main() -> i64 {
    let s: &[i64] = Vec::as_slice(mkv())
    return s.len
}
"#;

const I3_MUT_SLICE_OF_A_TEMPORARY: &str = r#"
fn mkv() -> Vec<i64> { return vec[1, 2, 3] }
fn main() -> i64 {
    let s: &mut [i64] = Vec::try_as_unique_mut_slice(mkv())
    return s.len
}
"#;

const I3_AS_SLICE_OF_A_LITERAL: &str = r#"
fn main() -> i64 {
    let s: &[i64] = Vec::as_slice(vec[1, 2, 3])
    return s.len
}
"#;

#[test]
fn ice3_a_view_into_a_temporary_is_refused_by_name() {
    let h = Harness::new();
    h.fenced_row("ICE-3a", I3_AS_SLICE_OF_A_TEMPORARY, "E0421", REPAIR);
    h.fenced_row("ICE-3b", I3_MUT_SLICE_OF_A_TEMPORARY, "E0421", REPAIR);
    h.fenced_row("ICE-3c", I3_AS_SLICE_OF_A_LITERAL, "E0421", REPAIR);
    h.assert_legs(9);
}

const I4_NESTED_RC_READ: &str = r#"
fn main() -> i64 {
    let rr: Rc<Rc<i64>> = Rc::new(Rc::new(7))
    println(Rc::get(Rc::get(rr)))
    return 0
}
"#;

#[test]
fn ice4_a_nested_rc_read_is_refused_by_name() {
    let h = Harness::new();
    h.fenced_row(
        "ICE-4",
        I4_NESTED_RC_READ,
        "E0410",
        "a nested `Rc` payload is not supported yet",
    );
    h.assert_legs(3);
}

const I5_LEN_ON_A_PLACE: &str = r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    println(Vec::len(v))
    Vec::push(v, 4)
    println(Vec::len(v))
    return 0
}
"#;

const I5_AS_SLICE_ON_A_PLACE: &str = r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    let s: &[i64] = Vec::as_slice(v)
    println(s.len)
    println(s[2])
    return 0
}
"#;

const I5_AS_SLICE_THROUGH_A_DEREF: &str = r#"
nogc fn read(v: &Vec<i64>) -> i64 {
    let s: &[i64] = Vec::as_slice(*v)
    return Vec::len(*v) + s[0]
}
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7, 2, 3]
    println(read(&v))
    return 0
}
"#;

const I5_RC_GET_ONE_LAYER: &str = r#"
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(101)
    println(Rc::get(r))
    return 0
}
"#;

#[test]
fn ice5_the_shapes_that_were_already_fine_are_untouched() {
    let h = Harness::new();
    h.value_row("ICE-5a", I5_LEN_ON_A_PLACE, "3\n4\n", 1, 1);
    h.value_row("ICE-5b", I5_AS_SLICE_ON_A_PLACE, "3\n3\n", 1, 1);
    h.value_row("ICE-5c", I5_AS_SLICE_THROUGH_A_DEREF, "10\n", 1, 1);
    h.value_row("ICE-5d", I5_RC_GET_ONE_LAYER, "101\n", 1, 1);
    h.assert_legs(24);
}

const I6_EVERY_INTERCEPT_ON_A_PLACE: &str = r#"
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(7)
    let z: Rc<i64> = Rc::null()
    let mut a: Vec<i64> = Vec::new()
    Vec::push(a, 11)
    Vec::push(a, 22)
    let mut b: Vec<i64> = vec[1, 2, 3]
    let s: &[i64] = Vec::as_slice(b)
    let mut c: Vec<i64> = vec[4, 5]
    let m: &mut [i64] = Vec::try_as_unique_mut_slice(c)
    m[0] = 40
    println(Rc::get(r))
    println(Vec::len(a))
    println(s.len)
    println(Vec::len(c))
    println(c[0])
    return 0
}
"#;

#[test]
fn ice6_every_builtin_intercept_still_answers_on_a_place_receiver() {
    let h = Harness::new();
    h.value_row(
        "ICE-6",
        I6_EVERY_INTERCEPT_ON_A_PLACE,
        "7\n2\n3\n2\n40\n",
        4,
        4,
    );
    h.assert_legs(6);
}

const I7_RC_NEW: &str = r#"
fn mki() -> i64 { return 7 }
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(mki())
    println(Rc::get(r))
    return 0
}
"#;

const I7_RC_GET: &str = r#"
fn mkr() -> Rc<i64> { return Rc::new(7) }
fn main() -> i64 {
    println(Rc::get(mkr()))
    return 0
}
"#;

const I7_RC_NULL: &str = r#"
fn mkz() -> Rc<i64> { return Rc::null() }
fn main() -> i64 {
    println(Rc::get(mkz()))
    return 0
}
"#;

const I7_VEC_NEW: &str = r#"
fn mkn() -> Vec<i64> { return Vec::new() }
fn main() -> i64 {
    println(Vec::len(mkn()))
    return 0
}
"#;

const I7_VEC_PUSH: &str = r#"
fn mkv() -> Vec<i64> { return vec[1, 2, 3] }
fn main() -> i64 {
    Vec::push(mkv(), 4)
    return 0
}
"#;

const I7_VEC_LEN: &str = r#"
fn mkv() -> Vec<i64> { return vec[1, 2, 3] }
fn main() -> i64 {
    return Vec::len(mkv())
}
"#;

#[test]
fn ice7_no_builtin_intercept_reports_a_compiler_bug_on_a_non_place_receiver() {
    let h = Harness::new();
    h.no_ice_row("ICE-7a", I7_RC_NEW);
    h.no_ice_row("ICE-7b", I7_RC_GET);
    h.no_ice_row("ICE-7c", I7_RC_NULL);
    h.no_ice_row("ICE-7d", I7_VEC_NEW);
    h.no_ice_row("ICE-7e", I7_VEC_PUSH);
    h.no_ice_row("ICE-7f", I7_VEC_LEN);
    h.no_ice_row("ICE-7g", I3_AS_SLICE_OF_A_TEMPORARY);
    h.no_ice_row("ICE-7h", I3_MUT_SLICE_OF_A_TEMPORARY);
    h.no_ice_row("ICE-7i", I4_NESTED_RC_READ);
    h.assert_legs(27);
}

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

    fn stage(&self, id: &str, tag: &str, files: &[(&str, &str)]) -> PathBuf {
        let dir = self.dir.path().join(slug(id, tag));
        fs::create_dir_all(&dir).expect("stage dir");
        for (name, body) in files {
            fs::write(dir.join(name), body).expect("write fixture");
        }
        dir.join("root.aelys")
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

    fn assert_row(&self, id: &str, root: &Path, tag: &str, stdout: &str, allocs: i64, frees: i64) {
        for (alloc_name, alloc) in ALLOCATORS {
            let o = self.run(root, *alloc);
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

    fn module_row(&self, id: &str, files: &[(&str, &str)], stdout: &str, allocs: i64, frees: i64) {
        for (tag, opt) in LEVELS {
            let root = self.stage(id, tag, files);
            let Some(exe) = self.compile_at(id, tag, &root, *opt) else {
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

const L1_ARRAY_VIEW: &str = r#"
fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    let s: &[i64] = a[..]
    println(s.len)
    return 0
}
"#;

#[test]
fn len1_a_view_of_an_array_answers_its_length() {
    let h = Harness::new();
    h.value_row("LEN-1", L1_ARRAY_VIEW, "3\n", 0, 0);
    h.assert_legs(6);
}

const L2_VEC_VIEW: &str = r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[10, 20, 30, 40]
    let n = Vec::len(v)
    let s: &[i64] = v[..]
    println(s.len)
    println(s.len - n)
    return 0
}
"#;

#[test]
fn len2_a_view_of_a_vec_answers_what_vec_len_answers() {
    let h = Harness::new();
    h.value_row("LEN-2", L2_VEC_VIEW, "4\n0\n", 1, 1);
    h.assert_legs(6);
}

const L3_MUT_VIEW: &str = r#"
nogc fn wcount(s: &mut [i64]) -> i64 {
    return s.len
}
fn main() -> i64 {
    let mut a: [i64;5] = [1, 2, 3, 4, 5]
    let m: &mut [i64] = a[..]
    println(m.len)
    println(wcount(m))
    return 0
}
"#;

#[test]
fn len3_mutability_does_not_gate_reading_a_length() {
    let h = Harness::new();
    h.value_row("LEN-3", L3_MUT_VIEW, "5\n5\n", 0, 0);
    h.assert_legs(6);
}

const L4_NOGC: &str = r#"
nogc fn count(s: &[i64]) -> i64 {
    return s.len
}
fn main() -> i64 {
    let a: [i64;7] = [1, 2, 3, 4, 5, 6, 7]
    let s: &[i64] = a[..]
    println(count(s))
    return 0
}
"#;

#[test]
fn len4_a_length_read_inside_nogc_allocates_nothing() {
    let h = Harness::new();
    h.value_row("LEN-4", L4_NOGC, "7\n", 0, 0);
    h.assert_legs(6);
}

const L5_MODULE: &[(&str, &str)] = &[
    (
        "root.aelys",
        r#"
needs lens

fn main() -> i64 {
    let a: [i64;5] = [1, 2, 3, 4, 5]
    let s: &[i64] = a[..]
    println(lens.count(s))
    return 0
}
"#,
    ),
    (
        "lens.aelys",
        r#"
pub nogc fn count(s: &[i64]) -> i64 {
    return s.len
}
"#,
    ),
];

#[test]
fn len5_a_length_survives_a_module_boundary() {
    let h = Harness::new();
    h.module_row("LEN-5", L5_MODULE, "5\n", 0, 0);
    h.assert_legs(6);
}

const L6_RESLICE: &str = r#"
fn main() -> i64 {
    let a: [i64;4] = [1, 2, 3, 4]
    let s: &[i64] = a[..]
    let t: &[i64] = s[..]
    println(t.len)
    println(t.len - s.len)
    return 0
}
"#;

#[test]
fn len6_a_reslice_reports_the_same_length() {
    let h = Harness::new();
    h.value_row("LEN-6", L6_RESLICE, "4\n0\n", 0, 0);
    h.assert_legs(6);
}

const L7_SCALAR: &str = r#"
fn main() -> i64 {
    let x: i64 = 7
    return x.len
}
"#;

const L7_UNKNOWN_ON_SLICE: &str = r#"
fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    let s: &[i64] = a[..]
    return s.cap
}
"#;

const L7_UNKNOWN_ON_VEC: &str = r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2]
    return v.cap
}
"#;

const L7_UNKNOWN_ON_ARRAY: &str = r#"
fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    return a.cap
}
"#;

#[test]
fn len7_the_length_did_not_open_a_field_access_hole() {
    let h = Harness::new();
    h.fenced_row("LEN-7a", L7_SCALAR, "E0304", "non-struct type i64");
    h.fenced_row(
        "LEN-7b",
        L7_UNKNOWN_ON_SLICE,
        "E0304",
        "unknown field 'cap' on &[i64]",
    );
    h.fenced_row(
        "LEN-7c",
        L7_UNKNOWN_ON_VEC,
        "E0304",
        "unknown field 'cap' on vec[i64]",
    );
    h.fenced_row(
        "LEN-7d",
        L7_UNKNOWN_ON_ARRAY,
        "E0304",
        "unknown field 'cap' on [i64; 3]",
    );
    h.assert_legs(12);
}

const L8_EMPTY: &str = r#"
fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    let e: &[i64] = a[0..0]
    println(e.len)
    return 0
}
"#;

#[test]
fn len8_an_empty_view_answers_zero() {
    let h = Harness::new();
    h.value_row("LEN-8", L8_EMPTY, "0\n", 0, 0);
    h.assert_legs(6);
}

const L9_NO_BINDING: &str = r#"
fn view(s: &[i64]) -> &[i64] { return s }
fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    let mut v: Vec<i64> = vec[10, 20, 30, 40, 50]
    let s: &[i64] = a[..]
    println(a[..].len)
    println(v[..].len)
    println(view(s).len)
    return 0
}
"#;

#[test]
fn len9_a_length_does_not_need_a_binding_first() {
    let h = Harness::new();
    h.value_row("LEN-9", L9_NO_BINDING, "3\n5\n3\n", 1, 1);
    h.assert_legs(6);
}

const L10_SUB_RANGE: &str = r#"
fn main() -> i64 {
    let a: [i64;5] = [1, 2, 3, 4, 5]
    let p: &[i64] = a[0..2]
    println(p.len)
    println(a[0..2].len)
    println(a[0..2].len - a[..].len)
    return 0
}
"#;

#[test]
fn len10_a_sub_range_reports_its_own_length_not_its_bases() {
    let h = Harness::new();
    h.value_row("LEN-10", L10_SUB_RANGE, "2\n2\n-3\n", 0, 0);
    h.assert_legs(6);
}

const L11_VEC_BOTH_WAYS: &str = r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[7, 7, 7, 7, 7, 7]
    println(v.len)
    println(Vec::len(v))
    println(v.len - Vec::len(v))
    Vec::push(v, 7)
    println(v.len)
    println(v.len - Vec::len(v))
    return 0
}
"#;

#[test]
fn len11_a_vec_answers_the_same_length_both_ways() {
    let h = Harness::new();
    h.value_row("LEN-11", L11_VEC_BOTH_WAYS, "6\n6\n0\n7\n0\n", 1, 1);
    h.assert_legs(6);
}

const L12_SIZED_ARRAY: &str = r#"
fn plen(a: [i64;4]) -> i64 { return a.len }
fn main() -> i64 {
    let a: [i64;4] = [9, 9, 9, 9]
    println(a.len)
    println(plen(a))
    println(a.len - plen(a))
    return 0
}
"#;

#[test]
fn len12_a_sized_array_answers_its_length_here_and_as_a_parameter() {
    let h = Harness::new();
    h.value_row("LEN-12", L12_SIZED_ARRAY, "4\n4\n0\n", 0, 0);
    h.assert_legs(6);
}

const L13_NOGC_ARRAY: &str = r#"
nogc fn alen(a: [i64;6]) -> i64 { return a.len }
nogc fn local() -> i64 {
    let b: [i64;2] = [1, 2]
    return b.len
}
fn main() -> i64 {
    let a: [i64;6] = [1, 2, 3, 4, 5, 6]
    println(alen(a))
    println(local())
    return 0
}
"#;

#[test]
fn len13_an_array_length_inside_nogc_allocates_nothing() {
    let h = Harness::new();
    h.value_row("LEN-13", L13_NOGC_ARRAY, "6\n2\n", 0, 0);
    h.assert_legs(6);
}

// a vec never crosses the nogc fence, so `.len` on one inside nogc has no reachable spelling
const L14_NOGC_VEC_PARAM: &str = r#"
nogc fn vlen(v: Vec<i64>) -> i64 { return v.len }
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    return vlen(v)
}
"#;

const L14_NOGC_VEC_LOCAL: &str = r#"
nogc fn body() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3]
    return v.len
}
fn main() -> i64 {
    return body()
}
"#;

#[test]
fn len14_a_vec_length_inside_nogc_is_refused_by_the_fence() {
    let h = Harness::new();
    h.fenced_row(
        "LEN-14a",
        L14_NOGC_VEC_PARAM,
        "E0727",
        "reach managed memory",
    );
    h.fenced_row(
        "LEN-14b",
        L14_NOGC_VEC_LOCAL,
        "E0727",
        "reach managed memory",
    );
    h.assert_legs(6);
}

const L15_NOGC_VEC_VIEW: &str = r#"
nogc fn vlen(s: &[i64]) -> i64 { return s.len }
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2, 3, 4, 5, 6, 7, 8]
    println(vlen(v[..]))
    return 0
}
"#;

#[test]
fn len15_a_vec_view_read_inside_nogc_adds_no_allocation() {
    let h = Harness::new();
    h.value_row("LEN-15", L15_NOGC_VEC_VIEW, "8\n", 1, 1);
    h.assert_legs(6);
}

const L16_STRING: &str = r#"
fn mk() -> string { return "abcd" }
fn main() -> i64 {
    let s: string = "hello"
    println(s.len)
    println("hi".len)
    println(mk().len)
    return 0
}
"#;

#[test]
fn len16_a_string_answers_its_length_from_any_receiver() {
    let h = Harness::new();
    h.value_row("LEN-16", L16_STRING, "5\n2\n4\n", 0, 0);
    h.assert_legs(6);
}

const L17_ADDR_STRING: &str = r#"
fn main() -> i64 {
    let mut s: string = "hello"
    let r = &s.len
    return 0
}
"#;

const L17_ADDR_SLICE: &str = r#"
fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    let mut sl: &[i64] = a[..]
    let r = &sl.len
    return 0
}
"#;

const L17_ADDR_VEC: &str = r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2]
    let r = &v.len
    return 0
}
"#;

const L17_ADDR_ARRAY: &str = r#"
fn main() -> i64 {
    let mut a: [i64;3] = [1, 2, 3]
    let r = &a.len
    return 0
}
"#;

#[test]
fn len17_a_length_has_no_address() {
    let h = Harness::new();
    let says = "there is no address to take";
    h.fenced_row("LEN-17a", L17_ADDR_STRING, "E0621", says);
    h.fenced_row("LEN-17b", L17_ADDR_SLICE, "E0621", says);
    h.fenced_row("LEN-17c", L17_ADDR_VEC, "E0621", says);
    h.fenced_row("LEN-17d", L17_ADDR_ARRAY, "E0621", says);
    h.assert_legs(12);
}

const L18_WRITE_STRING: &str = r#"
fn main() -> i64 {
    let mut s: string = "hello"
    s.len = 9
    return 0
}
"#;

const L18_WRITE_SLICE: &str = r#"
fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    let mut sl: &[i64] = a[..]
    sl.len = 9
    return 0
}
"#;

const L18_WRITE_VEC: &str = r#"
fn main() -> i64 {
    let mut v: Vec<i64> = vec[1, 2]
    v.len = 9
    return 0
}
"#;

const L18_WRITE_ARRAY: &str = r#"
fn main() -> i64 {
    let mut a: [i64;3] = [1, 2, 3]
    a.len = 9
    return 0
}
"#;

#[test]
fn len18_a_length_cannot_be_assigned() {
    let h = Harness::new();
    let says = "writing it would change nothing";
    h.fenced_row("LEN-18a", L18_WRITE_STRING, "E0621", says);
    h.fenced_row("LEN-18b", L18_WRITE_SLICE, "E0621", says);
    h.fenced_row("LEN-18c", L18_WRITE_VEC, "E0621", says);
    h.fenced_row("LEN-18d", L18_WRITE_ARRAY, "E0621", says);
    h.assert_legs(12);
}

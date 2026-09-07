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

const MODULES: &[&str] = &[
    "result.aelys",
    "slice.aelys",
    "sort.aelys",
    "vec.aelys",
    "str.aelys",
];

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

// the suite must compile the tracked library, so a missing tree is a failure and never a fallback
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

    // `needs std.vec` resolves under the root file, so the library is copied beside every root
    fn stage(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let dir = self.dir.path().join(slug(id, tag));
        fs::create_dir_all(&dir).expect("stage dir");
        copy_tree(&self.library, &dir.join("std"));
        let root = dir.join("root.aelys");
        fs::write(&root, src).expect("write root fixture");
        root
    }

    fn compile_at(&self, id: &str, tag: &str, root: &Path, opt: OptimizationLevel) -> Option<PathBuf> {
        match compile_file_with_llvm_variant(root, opt, false, RuntimeVariant::Rc) {
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

    fn row(&self, id: &str, src: &str, stdout: &str, stats: Option<(i64, i64)>) {
        for (tag, opt) in LEVELS {
            let root = self.stage(id, tag, src);
            let Some(exe) = self.compile_at(id, tag, &root, *opt) else {
                assert!(
                    linker_skip_declared(),
                    "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped value \
                     row carries no runtime evidence at all"
                );
                return;
            };
            for (alloc_name, alloc) in ALLOCATORS {
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
                    "{id} at {tag}/{alloc_name} must exit 0\nstderr:\n{seen_err}"
                );
                assert_eq!(
                    seen_out, stdout,
                    "{id} at {tag}/{alloc_name}: stdout MUST be {stdout:?}"
                );
                if let Some(want) = stats {
                    let got = parse_stats(&seen_err).unwrap_or_else(|| {
                        panic!("{id} at {tag}/{alloc_name}: no [rc] stats line\nstderr:\n{seen_err}")
                    });
                    assert_eq!(
                        got, want,
                        "{id} at {tag}/{alloc_name}: MUST be allocs={} frees={}",
                        want.0, want.1
                    );
                }
            }
        }
    }

    fn nogc_row(&self, id: &str, src: &str, stdout: &str) {
        self.row(id, src, stdout, Some((0, 0)));
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

    fn rejects(&self, id: &str, src: &str, code: &str, says: &str) {
        let root = self.stage(id, "reject", src);
        let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
            Ok(_) => panic!("{id}: MUST be rejected\n{src}"),
            Err(rendered) => rendered,
        };
        assert!(
            rendered.contains(&format!("[{code}]")),
            "{id}: the rejection MUST be {code}\n{src}\nrendered:\n{rendered}"
        );
        assert!(
            rendered.contains(says),
            "{id}: the rejection MUST say {says:?}\n{src}\nrendered:\n{rendered}"
        );
    }
}


const VEC_READS: &str = r#"
needs std.result
needs std.vec

fn main() -> i64 {
    let a: [i64;5] = [3, 1, 4, 1, 5]
    let v = vec.from_slice(a[..])
    println(vec.len(&v))
    if vec.is_empty(&v) { println(1) } else { println(0) }
    println(result.some_or(vec.get(&v, 2), -777))
    println(result.some_or(vec.get(&v, -1), -777))
    println(result.some_or(vec.get(&v, 5), -777))
    println(result.some_or(vec.first(&v), -777))
    println(result.some_or(vec.last(&v), -777))
    println(vec.index_of(&v, 4))
    println(vec.index_of(&v, 9))
    if vec.contains(&v, 5) { println(1) } else { println(0) }
    if vec.contains(&v, 9) { println(1) } else { println(0) }
    println(vec.sum(&v))
    if vec.eq(&v, &v) { println(1) } else { println(0) }
    if vec.is_sorted(&v) { println(1) } else { println(0) }
    return 0
}
"#;

#[test]
fn group_std_vec_reads_answer_and_allocate_only_the_source_vec() {
    let h = Harness::new();
    h.row(
        "STD-VEC-1",
        VEC_READS,
        "5\n0\n4\n-777\n-777\n3\n5\n2\n-1\n1\n0\n14\n1\n0\n",
        Some((1, 1)),
    );
    h.assert_legs(6);
}

const VEC_EMPTY: &str = r#"
needs std.result
needs std.vec

fn main() -> i64 {
    let mut z: [i64;0] = []
    let e = vec.from_slice(z[..])
    println(vec.len(&e))
    if vec.is_empty(&e) { println(1) } else { println(0) }
    if result.is_none(vec.get(&e, 0)) { println(1) } else { println(0) }
    if result.is_none(vec.first(&e)) { println(1) } else { println(0) }
    if result.is_none(vec.last(&e)) { println(1) } else { println(0) }
    println(vec.index_of(&e, 3))
    println(vec.sum(&e))
    if vec.is_sorted(&e) { println(1) } else { println(0) }
    println(vec.binary_search(&e, 3))
    if vec.eq(&e, &e) { println(1) } else { println(0) }
    return 0
}
"#;

#[test]
fn group_std_vec_answers_on_an_empty_vec_without_reading_out_of_bounds() {
    let h = Harness::new();
    h.row(
        "STD-VEC-2",
        VEC_EMPTY,
        "0\n1\n1\n1\n1\n-1\n0\n1\n-1\n1\n",
        Some((1, 1)),
    );
    h.assert_legs(6);
}

// eight bound vecs, so a builder that allocated twice or leaked one would move this count
const VEC_BUILDERS: &str = r#"
needs std.result
needs std.slice
needs std.vec

fn dbl(x: i64) -> i64 { return x * 2 }
fn odd(x: i64) -> bool { return x % 2 == 1 }
fn add(a: i64, b: i64) -> i64 { return a + b }

fn main() -> i64 {
    let a: [i64;5] = [3, 1, 4, 1, 5]
    let b: [i64;2] = [9, 2]
    let v = vec.from_slice(a[..])
    let w = vec.from_slice(b[..])
    let c = vec.copy(&v)
    let cc = vec.concat(&v, &w)
    let r = vec.reversed(&v)
    let s = vec.sorted(&v)
    let m = vec.map(&v, dbl)
    let f = vec.filter(&v, odd)
    println(vec.sum(&c))
    println(vec.len(&cc))
    println(vec.sum(&cc))
    println(result.some_or(vec.first(&r), -777))
    println(result.some_or(vec.last(&r), -777))
    if vec.is_sorted(&s) { println(1) } else { println(0) }
    println(vec.binary_search(&s, 4))
    println(vec.sum(&m))
    println(vec.len(&f))
    println(vec.sum(&f))
    println(vec.fold(&v, 100, add))
    if vec.eq(&v, &c) { println(1) } else { println(0) }
    if vec.eq(&v, &s) { println(1) } else { println(0) }
    println(slice.sum(Vec::as_slice(v)))
    return 0
}
"#;

#[test]
fn group_std_vec_builders_allocate_exactly_one_vec_each() {
    let h = Harness::new();
    h.row(
        "STD-VEC-3",
        VEC_BUILDERS,
        "14\n7\n25\n5\n3\n1\n3\n28\n4\n10\n114\n1\n0\n14\n",
        Some((8, 8)),
    );
    h.assert_legs(6);
}

// a defect of the compiler, not of the library: a `vec` consumed as a temporary is never dropped
const VEC_TEMPORARY: &str = r#"
needs std.vec

fn dbl(x: i64) -> i64 { return x * 2 }

fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    let v = vec.from_slice(a[..])
    println(Vec::len(vec.map(&v, dbl)))
    return 0
}
"#;

#[test]
fn group_std_vec_an_unbound_temporary_is_allocated_and_never_freed() {
    let h = Harness::new();
    h.row("STD-VEC-4", VEC_TEMPORARY, "3\n", Some((2, 1)));
    h.assert_legs(6);
}

const VEC_TEMPORARY_BOUND: &str = r#"
needs std.vec

fn dbl(x: i64) -> i64 { return x * 2 }

fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    let v = vec.from_slice(a[..])
    let m = vec.map(&v, dbl)
    println(vec.sum(&m))
    return 0
}
"#;

#[test]
fn group_std_vec_the_same_value_bound_to_a_local_is_freed() {
    let h = Harness::new();
    h.row("STD-VEC-5", VEC_TEMPORARY_BOUND, "12\n", Some((2, 2)));
    h.assert_legs(6);
}

const STR_PREDICATES: &str = r#"
needs std.result
needs std.str

fn main() -> i64 {
    let s: string = "hello world"
    if str.is_empty("") { println(1) } else { println(0) }
    if str.is_empty(s) { println(1) } else { println(0) }
    if str.starts_with(s, "hello") { println(1) } else { println(0) }
    if str.starts_with(s, "hellp") { println(1) } else { println(0) }
    if str.starts_with(s, "hello world!") { println(1) } else { println(0) }
    if str.starts_with(s, "") { println(1) } else { println(0) }
    if str.ends_with(s, "world") { println(1) } else { println(0) }
    if str.ends_with(s, "worle") { println(1) } else { println(0) }
    if str.ends_with(s, "") { println(1) } else { println(0) }
    println(str.index_of(s, "o"))
    println(str.index_of(s, "world"))
    println(str.index_of(s, "zz"))
    println(str.index_of(s, ""))
    println(str.index_of(s, "hello world!"))
    if str.contains(s, "lo wo") { println(1) } else { println(0) }
    if str.contains(s, "lo  wo") { println(1) } else { println(0) }
    println(result.some_or(str.char_at(s, 0), "?"))
    println(result.some_or(str.char_at(s, 10), "?"))
    if result.is_none(str.char_at(s, 11)) { println(1) } else { println(0) }
    if result.is_none(str.char_at(s, -1)) { println(1) } else { println(0) }
    if str.is_space(" ") { println(1) } else { println(0) }
    if str.is_space("x") { println(1) } else { println(0) }
    return 0
}
"#;

#[test]
fn group_std_str_predicates_answer_without_allocating() {
    let h = Harness::new();
    h.nogc_row(
        "STD-STR-1",
        STR_PREDICATES,
        "1\n0\n1\n0\n0\n1\n1\n0\n1\n4\n6\n-1\n0\n-1\n1\n0\nh\nd\n1\n1\n1\n0\n",
    );
    h.assert_legs(6);
}

const STR_NOGC: &str = r#"
needs std.result
needs std.str

nogc fn every_str_predicate(s: string) -> i64 {
    let mut acc: i64 = 0
    if str.is_empty(s) { acc = acc + 1 }
    if str.starts_with(s, "hel") { acc = acc + 2 }
    if str.ends_with(s, "rld") { acc = acc + 4 }
    if str.contains(s, "o w") { acc = acc + 8 }
    if str.is_space(" ") { acc = acc + 16 }
    acc = acc + str.index_of(s, "world")
    acc = acc + result.some_or(str.char_at(s, 0), "?").len
    return acc
}

fn main() -> i64 {
    println(every_str_predicate("hello world"))
    return 0
}
"#;

#[test]
fn group_std_str_reading_surface_is_callable_from_nogc() {
    let h = Harness::new();
    h.nogc_row("STD-STR-2", STR_NOGC, "37\n");
    h.assert_legs(6);
}

const STR_BUILDERS: &str = r#"
needs std.str

fn main() -> i64 {
    let s: string = "hello world"
    let a: string = str.substring(s, 0, 5)
    let b: string = str.substring(s, 6, 11)
    let c: string = str.substring(s, -3, 2)
    let d: string = str.substring(s, 3, 99)
    let e: string = str.substring(s, 4, 4)
    println(a)
    println(b)
    println(c)
    println(d)
    println("[" + e + "]")
    println(str.repeat("ab", 3))
    println("[" + str.repeat("ab", 0) + "]")
    println("[" + str.trim("  hi \t\n") + "]")
    println("[" + str.trim("   ") + "]")
    println("[" + str.trim("hi") + "]")
    return 0
}
"#;

#[test]
fn group_std_str_builders_answer_and_every_concatenation_is_counted() {
    let h = Harness::new();
    h.row(
        "STD-STR-3",
        STR_BUILDERS,
        "hello\nworld\nhe\nlo world\n[]\nababab\n[]\n[hi]\n[]\n[hi]\n",
        Some((37, 0)),
    );
    h.assert_legs(6);
}

// the four freed allocations are the four vecs; the other 23 are string concatenations, which never free
const STR_SPLIT_JOIN: &str = r#"
needs std.str
needs std.vec

fn main() -> i64 {
    let parts = str.split("a,b,,c", ",")
    println(Vec::len(parts))
    println(parts[0])
    println("[" + parts[2] + "]")
    println(parts[3])
    let j: string = str.join(&parts, "-")
    println(j)
    let one = str.split("abc", ",")
    println(Vec::len(one))
    println(one[0])
    let whole = str.split("abc", "")
    println(Vec::len(whole))
    println(whole[0])
    let multi = str.split("aXXbXXc", "XX")
    println(Vec::len(multi))
    println(multi[1])
    let k: string = str.join(&multi, "")
    println(k)
    return 0
}
"#;

#[test]
fn group_std_str_split_and_join_round_trip_and_keep_the_empty_parts() {
    let h = Harness::new();
    h.row(
        "STD-STR-4",
        STR_SPLIT_JOIN,
        "4\na\n[]\nc\na-b--c\n1\nabc\n1\nabc\n3\nb\nabc\n",
        Some((27, 4)),
    );
    h.assert_legs(6);
}

fn substring_growth(n: usize) -> String {
    format!(
        r#"
needs std.str

fn main() -> i64 {{
    let s: string = "0123456789abcdefghijklmnopqrstuvwxyz0123456789"
    let t: string = str.substring(s, 0, {n})
    println(t.len)
    return 0
}}
"#
    )
}

#[test]
fn group_std_str_substring_allocates_once_per_character_it_copies() {
    let h = Harness::new();
    for n in [4usize, 8, 16] {
        h.row(
            "STD-STR-5",
            &substring_growth(n),
            &format!("{n}\n"),
            Some((n as i64, 0)),
        );
    }
    h.assert_legs(18);
}

const STR_LEN_ADDRESS: &str = r#"
needs std.str

fn main() -> i64 {
    let s: string = str.trim("  hello  ")
    let n = &s.len
    return 0
}
"#;

const STR_LEN_WRITE: &str = r#"
needs std.str

fn main() -> i64 {
    let mut s: string = str.trim("  hello  ")
    s.len = 9
    return 0
}
"#;

#[test]
fn group_std_str_the_length_of_a_library_string_is_a_value_and_never_a_place() {
    let h = Harness::new();
    h.rejects(
        "STD-STR-6a",
        STR_LEN_ADDRESS,
        "E0621",
        "cannot take the address of `.len` on `string`",
    );
    h.rejects(
        "STD-STR-6b",
        STR_LEN_WRITE,
        "E0621",
        "cannot assign to `.len` on `string`",
    );
}

const VEC_TEMPORARY_VIEW: &str = r#"
needs std.vec

fn dbl(x: i64) -> i64 { return x * 2 }

fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    let v = vec.from_slice(a[..])
    let s: &[i64] = Vec::as_slice(vec.map(&v, dbl))
    return s[0]
}
"#;

#[test]
fn group_std_vec_a_slice_view_into_an_unbound_temporary_is_refused_by_name() {
    let h = Harness::new();
    h.rejects(
        "STD-VEC-6",
        VEC_TEMPORARY_VIEW,
        "E0421",
        "the receiver of Vec::as_slice denotes no storage",
    );
}

const STR_PARSE_INT: &str = r#"
needs std.result
needs std.str

nogc fn val(s: string) -> i64 { return result.unwrap_or(str.parse_int(s), -777) }
nogc fn code(s: string) -> i64 { return result.err_or(str.parse_int(s), 0) }

fn main() -> i64 {
    println(val("0"))
    println(val("-0"))
    println(val("42"))
    println(val("-42"))
    println(val("007"))
    println(val("9223372036854775807"))
    println(val("-9223372036854775808"))
    println(code("0"))
    println(code(""))
    println(code("-"))
    println(code("1a"))
    println(code("a"))
    println(code("+1"))
    println(code(" 1"))
    println(code("9223372036854775808"))
    println(code("-9223372036854775809"))
    println(code("99999999999999999999"))
    return 0
}
"#;

#[test]
fn group_std_str_parse_int_answers_and_names_every_way_it_can_fail() {
    let h = Harness::new();
    h.nogc_row(
        "STD-STR-7",
        STR_PARSE_INT,
        "0\n0\n42\n-42\n7\n9223372036854775807\n-9223372036854775808\n0\n1\n2\n3\n3\n3\n3\n4\n4\n4\n",
    );
    h.assert_legs(6);
}

const STR_INT_ROUND_TRIP: &str = r#"
needs std.result
needs std.str

fn trip(n: i64) -> bool {
    let s: string = str.from_int(n)
    let back: i64 = result.unwrap_or(str.parse_int(s), -777)
    return back == n
}

fn main() -> i64 {
    let min: i64 = 0 - 9223372036854775807 - 1
    println(str.from_int(0))
    println(str.from_int(-42))
    println(str.from_int(9223372036854775807))
    println(str.from_int(min))
    if trip(0) { println(1) } else { println(0) }
    if trip(7) { println(1) } else { println(0) }
    if trip(-7) { println(1) } else { println(0) }
    if trip(9223372036854775807) { println(1) } else { println(0) }
    if trip(min) { println(1) } else { println(0) }
    return 0
}
"#;

// nine interpolations and the count is still zero: they lower to __aelys_to_string_i64, which mallocs outside the rc allocator and never frees
#[test]
fn group_std_str_from_int_and_parse_int_round_trip_across_the_whole_range() {
    let h = Harness::new();
    h.row(
        "STD-STR-8",
        STR_INT_ROUND_TRIP,
        "0\n-42\n9223372036854775807\n-9223372036854775808\n1\n1\n1\n1\n1\n",
        Some((0, 0)),
    );
    h.assert_legs(6);
}

const STR_PARSE_UNUSED: &str = r#"
needs std.str

fn main() -> i64 {
    str.parse_int("1")
    return 0
}
"#;

const STR_PARSE_DISCARDED: &str = r#"
needs std.str

fn main() -> i64 {
    discard str.parse_int("1")
    println(0)
    return 0
}
"#;

#[test]
fn group_std_str_a_parsed_result_is_must_use_and_discard_is_the_way_to_drop_it() {
    let h = Harness::new();
    h.rejects(
        "STD-STR-9a",
        STR_PARSE_UNUSED,
        "E0411",
        "this `Result` can fail with `i64`",
    );
    h.nogc_row("STD-STR-9b", STR_PARSE_DISCARDED, "0\n");
    h.assert_legs(6);
}

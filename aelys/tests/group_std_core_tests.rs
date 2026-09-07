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

const MODULES: &[&str] = &["result.aelys", "math.aelys", "slice.aelys", "sort.aelys"];

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

    // `needs std.math` resolves under the root file, so the library is copied beside every root
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

    fn value_row(&self, id: &str, src: &str, stdout: &str) {
        self.row(id, src, stdout, None);
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

const RESULT_PREDICATES: &str = r#"
needs std.result

fn main() -> i64 {
    let a: result.Result<i64, i64> = result.Result::Ok(7)
    let b: result.Result<i64, i64> = result.Result::Err(3)
    let s: result.Option<i64> = result.Option::Some(9)
    let n: result.Option<i64> = result.Option::None
    if result.is_ok(a) { println(1) } else { println(0) }
    if result.is_ok(b) { println(1) } else { println(0) }
    if result.is_err(a) { println(1) } else { println(0) }
    if result.is_err(b) { println(1) } else { println(0) }
    if result.is_some(s) { println(1) } else { println(0) }
    if result.is_some(n) { println(1) } else { println(0) }
    if result.is_none(s) { println(1) } else { println(0) }
    if result.is_none(n) { println(1) } else { println(0) }
    return 0
}
"#;

#[test]
fn group_std_result_predicates_answer_on_both_arms() {
    let h = Harness::new();
    h.value_row(
        "STD-RESULT-1",
        RESULT_PREDICATES,
        "1\n0\n0\n1\n1\n0\n0\n1\n",
    );
    h.assert_legs(6);
}

const RESULT_DEFAULTS: &str = r#"
needs std.result

fn main() -> i64 {
    let a: result.Result<i64, i64> = result.Result::Ok(7)
    let b: result.Result<i64, i64> = result.Result::Err(3)
    let s: result.Option<i64> = result.Option::Some(9)
    let n: result.Option<i64> = result.Option::None
    println(result.unwrap_or(a, -1))
    println(result.unwrap_or(b, -1))
    println(result.err_or(a, -2))
    println(result.err_or(b, -2))
    println(result.some_or(s, -3))
    println(result.some_or(n, -3))
    return 0
}
"#;

#[test]
fn group_std_result_defaults_answer_the_payload_or_the_fallback() {
    let h = Harness::new();
    h.value_row("STD-RESULT-2", RESULT_DEFAULTS, "7\n-1\n-2\n3\n9\n-3\n");
    h.assert_legs(6);
}

const RESULT_QUESTION_MARK: &str = r#"
needs std.result

fn checked(x: i64) -> result.Result<i64, i64> {
    if x > 0 { return result.Result::Ok(x) } else { return result.Result::Err(-x) }
}

fn twice(x: i64) -> result.Result<i64, i64> {
    let v = checked(x)?
    return result.Result::Ok(v * 2)
}

fn main() -> i64 {
    println(result.unwrap_or(twice(21), -1))
    println(result.unwrap_or(twice(-4), -1))
    println(result.err_or(twice(-4), -1))
    return 0
}
"#;

#[test]
fn group_std_result_carries_the_question_mark_across_the_module_boundary() {
    let h = Harness::new();
    h.value_row("STD-RESULT-3", RESULT_QUESTION_MARK, "42\n-1\n4\n");
    h.assert_legs(6);
}

const MATH_INTEGER: &str = r#"
needs std.math

fn main() -> i64 {
    println(math.abs(-7))
    println(math.abs(0))
    println(math.abs(7))
    println(math.min(3, 9))
    println(math.max(3, 9))
    println(math.clamp(-5, 0, 10))
    println(math.clamp(5, 0, 10))
    println(math.clamp(50, 0, 10))
    println(math.sign(-3))
    println(math.sign(0))
    println(math.sign(3))
    return 0
}
"#;

#[test]
fn group_std_math_integer_surface_answers() {
    let h = Harness::new();
    h.value_row(
        "STD-MATH-1",
        MATH_INTEGER,
        "7\n0\n7\n3\n9\n0\n5\n10\n-1\n0\n1\n",
    );
    h.assert_legs(6);
}

const MATH_POW_GCD_LCM: &str = r#"
needs std.math

fn main() -> i64 {
    println(math.pow(2, 10))
    println(math.pow(5, 0))
    println(math.pow(0, 0))
    println(math.pow(-2, 3))
    println(math.pow(2, -1))
    println(math.pow(1, -7))
    println(math.pow(-1, -7))
    println(math.pow(-1, -8))
    println(math.gcd(12, 18))
    println(math.gcd(0, 0))
    println(math.gcd(0, 5))
    println(math.gcd(-12, 18))
    println(math.lcm(4, 6))
    println(math.lcm(0, 5))
    println(math.lcm(-4, 6))
    return 0
}
"#;

#[test]
fn group_std_math_pow_gcd_and_lcm_answer_including_the_degenerate_inputs() {
    let h = Harness::new();
    h.value_row(
        "STD-MATH-2",
        MATH_POW_GCD_LCM,
        "1024\n1\n1\n-8\n0\n1\n-1\n1\n6\n0\n5\n6\n12\n0\n12\n",
    );
    h.assert_legs(6);
}

const MATH_FLOAT: &str = r#"
needs std.math

fn main() -> i64 {
    println(math.abs_f(-1.5))
    println(math.abs_f(2.25))
    println(math.min_f(1.5, 0.25))
    println(math.max_f(1.5, 0.25))
    return 0
}
"#;

#[test]
fn group_std_math_float_surface_answers() {
    let h = Harness::new();
    h.value_row("STD-MATH-3", MATH_FLOAT, "1.5\n2.25\n0.25\n1.5\n");
    h.assert_legs(6);
}

const SLICE_READS: &str = r#"
needs std.result
needs std.slice

fn main() -> i64 {
    let a: [i64;5] = [4, 1, 9, 3, 9]
    let b: [i64;4] = [4, 1, 9, 3]
    let c: [i64;5] = [4, 1, 9, 3, 8]
    let d: [i64;4] = [1, 2, 2, 5]
    println(slice.sum(a[..]))
    println(result.some_or(slice.max(a[..]), -777))
    println(result.some_or(slice.min(a[..]), -777))
    println(slice.index_of(a[..], 9))
    println(slice.index_of(a[..], 7))
    if slice.contains(a[..], 3) { println(1) } else { println(0) }
    if slice.contains(a[..], 7) { println(1) } else { println(0) }
    if slice.eq(a[..], a[..]) { println(1) } else { println(0) }
    if slice.eq(a[..], b[..]) { println(1) } else { println(0) }
    if slice.eq(a[..], c[..]) { println(1) } else { println(0) }
    if slice.is_sorted(a[..]) { println(1) } else { println(0) }
    if slice.is_sorted(d[..]) { println(1) } else { println(0) }
    return 0
}
"#;

#[test]
fn group_std_slice_reads_answer() {
    let h = Harness::new();
    h.value_row(
        "STD-SLICE-1",
        SLICE_READS,
        "26\n9\n1\n2\n-1\n1\n0\n1\n0\n0\n0\n1\n",
    );
    h.assert_legs(6);
}

const SLICE_EMPTY: &str = r#"
needs std.slice
needs std.sort

fn main() -> i64 {
    let mut e: [i64;0] = []
    let one: [i64;1] = [5]
    println(slice.sum(e[..]))
    println(slice.index_of(e[..], 3))
    if slice.contains(e[..], 3) { println(1) } else { println(0) }
    if slice.eq(e[..], e[..]) { println(1) } else { println(0) }
    if slice.is_sorted(e[..]) { println(1) } else { println(0) }
    if slice.is_sorted(one[..]) { println(1) } else { println(0) }
    println(sort.binary_search(e[..], 3))
    sort.insertion_sort(e[..])
    slice.reverse(e[..])
    println(slice.sum(e[..]))
    return 0
}
"#;

#[test]
fn group_std_slice_and_sort_answer_the_identities_on_an_empty_slice() {
    let h = Harness::new();
    h.value_row(
        "STD-SLICE-2",
        SLICE_EMPTY,
        "0\n-1\n0\n1\n1\n1\n-1\n0\n",
    );
    h.assert_legs(6);
}

const SLICE_OPTIONS: &str = r#"
needs std.result
needs std.slice

fn main() -> i64 {
    let a: [i64;5] = [4, 1, 9, 3, 9]
    let mut e: [i64;0] = []
    let floor: [i64;1] = [-9223372036854775807 - 1]
    let ceil: [i64;1] = [9223372036854775807]
    println(result.some_or(slice.max(a[..]), -777))
    println(result.some_or(slice.min(a[..]), -777))
    println(result.some_or(slice.first(a[..]), -777))
    println(result.some_or(slice.last(a[..]), -777))
    if result.is_none(slice.max(e[..])) { println(1) } else { println(0) }
    if result.is_none(slice.min(e[..])) { println(1) } else { println(0) }
    if result.is_none(slice.first(e[..])) { println(1) } else { println(0) }
    if result.is_none(slice.last(e[..])) { println(1) } else { println(0) }
    if result.is_some(slice.max(floor[..])) { println(1) } else { println(0) }
    println(result.some_or(slice.max(floor[..]), -777))
    if result.is_some(slice.min(ceil[..])) { println(1) } else { println(0) }
    println(result.some_or(slice.min(ceil[..]), -777))
    return 0
}
"#;

#[test]
fn group_std_slice_option_reads_separate_the_empty_slice_from_the_extreme_element() {
    let h = Harness::new();
    h.nogc_row(
        "STD-SLICE-4",
        SLICE_OPTIONS,
        "9\n1\n4\n9\n1\n1\n1\n1\n1\n-9223372036854775808\n1\n9223372036854775807\n",
    );
    h.assert_legs(6);
}

const SLICE_WRITES: &str = r#"
needs std.slice

fn main() -> i64 {
    let mut c: [i64;5] = [1, 2, 3, 4, 5]
    slice.swap(c[..], 0, 4)
    for i in 0..5 { println(c[i]) }
    slice.swap(c[..], 2, 2)
    println(c[2])
    slice.reverse(c[..])
    for i in 0..5 { println(c[i]) }
    let mut d: [i64;3] = [0, 0, 0]
    slice.fill(d[..], 7)
    println(slice.sum(d[..]))
    return 0
}
"#;

#[test]
fn group_std_slice_writes_land_in_the_caller_array() {
    let h = Harness::new();
    h.value_row(
        "STD-SLICE-3",
        SLICE_WRITES,
        "5\n2\n3\n4\n1\n3\n1\n4\n3\n2\n5\n21\n",
    );
    h.assert_legs(6);
}

const SORT_IN_PLACE: &str = r#"
needs std.slice
needs std.sort

fn main() -> i64 {
    let mut a: [i64;6] = [9, 5, 7, 1, 3, 5]
    sort.insertion_sort(a[..])
    for i in 0..6 { println(a[i]) }
    if slice.is_sorted(a[..]) { println(1) } else { println(0) }
    let mut b: [i64;4] = [1, 2, 3, 4]
    sort.insertion_sort(b[..])
    for i in 0..4 { println(b[i]) }
    let mut s: [i64;1] = [8]
    sort.insertion_sort(s[..])
    println(s[0])
    let mut r: [i64;5] = [5, 4, 3, 2, 1]
    sort.insertion_sort(r[..])
    println(slice.index_of(r[..], 1))
    println(slice.index_of(r[..], 5))
    return 0
}
"#;

#[test]
fn group_std_sort_orders_in_place_and_keeps_duplicates() {
    let h = Harness::new();
    h.value_row(
        "STD-SORT-1",
        SORT_IN_PLACE,
        "1\n3\n5\n5\n7\n9\n1\n1\n2\n3\n4\n8\n0\n4\n",
    );
    h.assert_legs(6);
}

const SORT_BINARY_SEARCH: &str = r#"
needs std.sort

fn main() -> i64 {
    let a: [i64;5] = [1, 3, 5, 7, 9]
    println(sort.binary_search(a[..], 1))
    println(sort.binary_search(a[..], 3))
    println(sort.binary_search(a[..], 5))
    println(sort.binary_search(a[..], 7))
    println(sort.binary_search(a[..], 9))
    println(sort.binary_search(a[..], 0))
    println(sort.binary_search(a[..], 4))
    println(sort.binary_search(a[..], 10))
    return 0
}
"#;

#[test]
fn group_std_sort_binary_search_answers_the_index_or_the_sentinel() {
    let h = Harness::new();
    h.value_row(
        "STD-SORT-2",
        SORT_BINARY_SEARCH,
        "0\n1\n2\n3\n4\n-1\n-1\n-1\n",
    );
    h.assert_legs(6);
}

const NOGC_MATH: &str = r#"
needs std.math

nogc fn every_math() -> i64 {
    let mut acc: i64 = 0
    acc = acc + math.abs(-7)
    acc = acc + math.min(4, 9)
    acc = acc + math.max(4, 9)
    acc = acc + math.clamp(20, 0, 10)
    acc = acc + math.sign(-3)
    acc = acc + math.pow(3, 4)
    acc = acc + math.gcd(24, 36)
    acc = acc + math.lcm(6, 8)
    acc = acc + (math.abs_f(-1.5) as i64)
    acc = acc + (math.min_f(2.0, 5.0) as i64)
    acc = acc + (math.max_f(2.0, 5.0) as i64)
    return acc
}

fn main() -> i64 {
    println(every_math())
    return 0
}
"#;

#[test]
fn group_std_math_is_callable_from_nogc_and_allocates_nothing() {
    let h = Harness::new();
    h.nogc_row("STD-NOGC-MATH", NOGC_MATH, "154\n");
    h.assert_legs(6);
}

const NOGC_SLICE: &str = r#"
needs std.result
needs std.slice

nogc fn every_slice(r: &[i64], w: &mut [i64]) -> i64 {
    let mut acc: i64 = 0
    acc = acc + slice.sum(r)
    acc = acc + result.some_or(slice.max(r), 0)
    acc = acc + result.some_or(slice.min(r), 0)
    acc = acc + slice.index_of(r, 3)
    if slice.contains(r, 3) { acc = acc + 1 }
    if slice.eq(r, r) { acc = acc + 1 }
    if slice.is_sorted(r) { acc = acc + 1 }
    slice.fill(w, 2)
    slice.swap(w, 0, 3)
    slice.reverse(w)
    acc = acc + slice.sum(w)
    return acc
}

fn main() -> i64 {
    let a: [i64;5] = [1, 2, 3, 4, 5]
    let mut b: [i64;4] = [0, 0, 0, 0]
    println(every_slice(a[..], b[..]))
    return 0
}
"#;

#[test]
fn group_std_slice_is_callable_from_nogc_and_allocates_nothing() {
    let h = Harness::new();
    h.nogc_row("STD-NOGC-SLICE", NOGC_SLICE, "34\n");
    h.assert_legs(6);
}

const NOGC_SORT: &str = r#"
needs std.sort

nogc fn every_sort(w: &mut [i64]) -> i64 {
    sort.insertion_sort(w)
    return sort.binary_search(w, 5) + sort.binary_search(w, 4)
}

fn main() -> i64 {
    let mut c: [i64;5] = [9, 5, 7, 1, 3]
    println(every_sort(c[..]))
    return 0
}
"#;

#[test]
fn group_std_sort_is_callable_from_nogc_and_allocates_nothing() {
    let h = Harness::new();
    h.nogc_row("STD-NOGC-SORT", NOGC_SORT, "1\n");
    h.assert_legs(6);
}

const NOGC_RESULT: &str = r#"
needs std.result

nogc fn every_result(a: result.Result<i64, i64>, b: result.Result<i64, i64>, s: result.Option<i64>, n: result.Option<i64>) -> i64 {
    let mut acc: i64 = 0
    if result.is_ok(a) { acc = acc + 1 }
    if result.is_err(b) { acc = acc + 2 }
    if result.is_some(s) { acc = acc + 4 }
    if result.is_none(n) { acc = acc + 8 }
    acc = acc + result.unwrap_or(a, -1)
    acc = acc + result.err_or(b, -1)
    acc = acc + result.some_or(s, -1)
    return acc
}

fn main() -> i64 {
    println(every_result(result.Result::Ok(7), result.Result::Err(3), result.Option::Some(9), result.Option::None))
    return 0
}
"#;

#[test]
fn group_std_result_is_callable_from_nogc_and_allocates_nothing() {
    let h = Harness::new();
    h.nogc_row("STD-NOGC-RESULT", NOGC_RESULT, "34\n");
    h.assert_legs(6);
}

// `result.result::ok(s[i])` is refused by e0714, so the element is bound before it is wrapped
const COMPOSITION: &str = r#"
needs std.math
needs std.slice
needs std.sort
needs std.result

fn median(s: &mut [i64]) -> result.Result<i64, i64> {
    if s.len == 0 { return result.Result::Err(-1) }
    sort.insertion_sort(s)
    let m: i64 = s[s.len / 2]
    return result.Result::Ok(m)
}

fn main() -> i64 {
    let mut a: [i64;5] = [9, 1, 7, 3, 5]
    println(result.unwrap_or(median(a[..]), -1))
    let mut e: [i64;0] = []
    println(result.err_or(median(e[..]), 0))
    println(math.gcd(result.some_or(slice.max(a[..]), 0), result.some_or(slice.min(a[..]), 0)))
    println(math.lcm(slice.sum(a[..]), 5))
    println(sort.binary_search(a[..], math.clamp(100, 1, 9)))
    return 0
}
"#;

#[test]
fn group_std_four_modules_compose_in_one_program_without_allocating() {
    let h = Harness::new();
    h.nogc_row("STD-COMPOSE", COMPOSITION, "5\n-1\n1\n25\n4\n");
    h.assert_legs(6);
}

// no generic combinator and no `.len`, so the only obstacle this row can meet is the `?` repair
const CARRIER_QUESTION_MARK_ALONE: &str = r#"
needs std.result

fn checked(x: i64) -> result.Result<i64, i64> {
    if x > 0 { return result.Result::Ok(x) } else { return result.Result::Err(0 - x) }
}

fn twice(x: i64) -> result.Result<i64, i64> {
    let v = checked(x)?
    return result.Result::Ok(v * 2)
}

fn show(r: result.Result<i64, i64>) -> i64 {
    return match r {
        result.Result::Ok(v) => v,
        result.Result::Err(e) => 0 - e,
    }
}

fn main() -> i64 {
    println(show(twice(21)))
    println(show(twice(-4)))
    return 0
}
"#;

#[test]
fn group_std_result_the_question_mark_crosses_the_boundary_with_no_combinator_in_the_way() {
    let h = Harness::new();
    h.value_row("STD-RESULT-6", CARRIER_QUESTION_MARK_ALONE, "42\n-4\n");
    h.assert_legs(6);
}

const CARRIER_CATCH_ALONE: &str = r#"
needs std.result

fn checked(x: i64) -> result.Result<i64, i64> {
    if x > 0 { return result.Result::Ok(x) } else { return result.Result::Err(0 - x) }
}

fn main() -> i64 {
    println(checked(8) catch |e| 90)
    println(checked(-9) catch |e| e + 1)
    return 0
}
"#;

#[test]
fn group_std_result_catch_reads_the_library_carrier_on_both_arms() {
    let h = Harness::new();
    h.value_row("STD-RESULT-7", CARRIER_CATCH_ALONE, "8\n10\n");
    h.assert_legs(6);
}

const CARRIER_METHODS_ALONE: &str = r#"
needs std.result

fn checked(x: i64) -> result.Result<i64, i64> {
    if x > 0 { return result.Result::Ok(x) } else { return result.Result::Err(0 - x) }
}

fn main() -> i64 {
    println(checked(8).unwrap())
    println(checked(5).expect("must be positive"))
    return 0
}
"#;

#[test]
fn group_std_result_unwrap_and_expect_read_the_library_carrier() {
    let h = Harness::new();
    h.value_row("STD-RESULT-8", CARRIER_METHODS_ALONE, "8\n5\n");
    h.assert_legs(6);
}

const CARRIER_SECOND_RESULT: &str = r#"
needs std.result

enum Result<T, E> { Ok(T), Err(E) }

fn checked(x: i64) -> result.Result<i64, i64> {
    if x > 0 { return result.Result::Ok(x) } else { return result.Result::Err(0 - x) }
}

fn run(x: i64) -> Result<i64, i64> {
    let v = checked(x)?
    return Result::Ok(v)
}

fn main() -> i64 {
    return 0
}
"#;

#[test]
fn group_std_result_a_local_carrier_meeting_the_library_one_at_a_question_mark_is_e0619() {
    let h = Harness::new();
    h.rejects(
        "STD-RESULT-9",
        CARRIER_SECOND_RESULT,
        "E0619",
        "`?` cannot bridge `std.result.Result<i64, i64>` and `Result<i64, i64>`",
    );
}

const CARRIER_QUESTION_MARK_IN_I64: &str = r#"
needs std.result

fn checked(x: i64) -> result.Result<i64, i64> {
    if x > 0 { return result.Result::Ok(x) } else { return result.Result::Err(0 - x) }
}

fn main() -> i64 {
    let v = checked(3)?
    return v
}
"#;

#[test]
fn group_std_result_a_question_mark_in_a_function_returning_i64_is_e0620() {
    let h = Harness::new();
    h.rejects(
        "STD-RESULT-10",
        CARRIER_QUESTION_MARK_IN_I64,
        "E0620",
        "`?` on a `Result` requires the function to return `Result<_, E>`",
    );
}

const CARRIER_DROPPED: &str = r#"
needs std.result

fn checked(x: i64) -> result.Result<i64, i64> {
    if x > 0 { return result.Result::Ok(x) } else { return result.Result::Err(0 - x) }
}

fn main() -> i64 {
    checked(3)
    return 0
}
"#;

#[test]
fn group_std_result_the_library_carrier_in_statement_position_is_e0411() {
    let h = Harness::new();
    h.rejects(
        "STD-RESULT-11",
        CARRIER_DROPPED,
        "E0411",
        "[must-use] this `Result` can fail with `i64`",
    );
}

const CARRIER_DISCARDED: &str = r#"
needs std.result

fn checked(x: i64) -> result.Result<i64, i64> {
    if x > 0 { return result.Result::Ok(x) } else { return result.Result::Err(0 - x) }
}

fn main() -> i64 {
    discard checked(3)
    println(7)
    return 0
}
"#;

#[test]
fn group_std_result_a_discarded_library_carrier_is_still_accepted() {
    let h = Harness::new();
    h.value_row("STD-RESULT-12", CARRIER_DISCARDED, "7\n");
    h.assert_legs(6);
}

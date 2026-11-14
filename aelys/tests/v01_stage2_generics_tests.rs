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
    ("-O1", OptimizationLevel::Basic),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

const ALLOCATORS: &[(&str, Option<&str>)] = &[("immix", None), ("malloc", Some("malloc"))];

const MODULES: &[&str] = &["result.aelys", "slice.aelys", "sort.aelys", "str.aelys"];

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

    // `needs std.slice` resolves under the root file, so the library is copied beside every root
    fn stage(&self, id: &str, tag: &str, src: &str) -> PathBuf {
        let dir = self.dir.path().join(slug(id, tag));
        fs::create_dir_all(&dir).expect("stage dir");
        copy_tree(&self.library, &dir.join("std"));
        let root = dir.join("root.aelys");
        fs::write(&root, src).expect("write root fixture");
        root
    }

    fn value_row(&self, id: &str, src: &str, stdout: &str) {
        for (tag, opt) in LEVELS {
            let root = self.stage(id, tag, src);
            match compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::Rc) {
                Ok(()) => {}
                Err(err) => {
                    if linker_unavailable(&err.to_string()) {
                        self.linker_skips.set(self.linker_skips.get() + 1);
                        assert!(
                            linker_skip_declared(),
                            "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped \
                             value row carries no runtime evidence at all"
                        );
                        return;
                    }
                    panic!("{id} at {tag} must compile:\n{err}");
                }
            }
            let exe = exe_path_for(&root);
            assert!(exe.is_file(), "{id} at {tag}: no artifact was written");
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

    // the refusal has to be the same one at every level, so the verdict never depends on `-o`
    fn rejects_at_every_level(&self, id: &str, src: &str, code: &str, says: &[&str]) {
        for (tag, opt) in LEVELS {
            let root = self.stage(id, tag, src);
            let rendered = match lower_file_to_air(&root, *opt) {
                Ok(_) => panic!("{id} at {tag}: MUST be rejected\n{src}"),
                Err(rendered) => rendered,
            };
            assert!(
                rendered.contains(&format!("[{code}]")),
                "{id} at {tag}: the rejection MUST be {code}\n{src}\nrendered:\n{rendered}"
            );
            for want in says {
                assert!(
                    rendered.contains(want),
                    "{id} at {tag}: the rejection MUST say {want:?}\n{src}\nrendered:\n{rendered}"
                );
            }
        }
    }

    fn accepts_once(&self, id: &str, src: &str) -> bool {
        let root = self.stage(id, "probe", src);
        lower_file_to_air(&root, OptimizationLevel::None).is_ok()
    }
}

const LIBRARY_ON_I64: &str = r#"
needs std.result
needs std.slice
needs std.sort

fn main() -> i64 {
    let a: [i64;5] = [3, 1, 4, 1, 5]
    println(slice.sum(a[..]))
    println(result.some_or(slice.first(a[..]), -777))
    println(result.some_or(slice.last(a[..]), -777))
    println(result.some_or(slice.max(a[..]), -777))
    println(result.some_or(slice.min(a[..]), -777))
    println(slice.index_of(a[..], 4))
    println(slice.index_of(a[..], 9))
    println(result.some_or(slice.find(a[..], 5), -777))
    if result.is_none(slice.find(a[..], 9)) { println(1) } else { println(0) }
    if slice.contains(a[..], 1) { println(1) } else { println(0) }
    if slice.eq(a[..], a[..]) { println(1) } else { println(0) }
    if slice.is_sorted(a[..]) { println(1) } else { println(0) }
    let mut b: [i64;5] = [3, 1, 4, 1, 5]
    sort.insertion_sort(b[..])
    if slice.is_sorted(b[..]) { println(1) } else { println(0) }
    println(sort.binary_search(b[..], 4))
    println(sort.binary_search(b[..], 9))
    slice.reverse(b[..])
    println(result.some_or(slice.first(b[..]), -777))
    slice.swap(b[..], 0, 4)
    println(result.some_or(slice.first(b[..]), -777))
    slice.fill(b[..], 7)
    println(slice.sum(b[..]))
    return 0
}
"#;

#[test]
fn s2_the_generic_library_answers_on_i64_at_every_level() {
    let h = Harness::new();
    h.value_row(
        "S2-LIB-I64",
        LIBRARY_ON_I64,
        "14\n3\n5\n5\n1\n2\n-1\n4\n1\n1\n1\n0\n1\n3\n-1\n5\n1\n35\n",
    );
    h.assert_legs(8);
}

// the same entry points, on a second element type the old library could not name at all
const LIBRARY_ON_CHAR: &str = r#"
needs std.result
needs std.slice
needs std.sort

fn main() -> i64 {
    let c: [char;5] = ['d', 'a', 'e', 'a', 'c']
    println(result.some_or(slice.first(c[..]), 'z'))
    println(result.some_or(slice.last(c[..]), 'z'))
    println(result.some_or(slice.max(c[..]), 'z'))
    println(result.some_or(slice.min(c[..]), 'z'))
    println(slice.index_of(c[..], 'e'))
    println(slice.index_of(c[..], 'q'))
    println(result.some_or(slice.find(c[..], 'c'), -777))
    if result.is_none(slice.find(c[..], 'q')) { println(1) } else { println(0) }
    if slice.contains(c[..], 'a') { println(1) } else { println(0) }
    if slice.eq(c[..], c[..]) { println(1) } else { println(0) }
    if slice.is_sorted(c[..]) { println(1) } else { println(0) }
    let mut d: [char;5] = ['d', 'a', 'e', 'a', 'c']
    sort.insertion_sort(d[..])
    if slice.is_sorted(d[..]) { println(1) } else { println(0) }
    println(sort.binary_search(d[..], 'd'))
    println(sort.binary_search(d[..], 'q'))
    slice.reverse(d[..])
    println(result.some_or(slice.first(d[..]), 'z'))
    slice.swap(d[..], 0, 4)
    println(result.some_or(slice.first(d[..]), 'z'))
    slice.fill(d[..], 'q')
    println(result.some_or(slice.last(d[..]), 'z'))
    return 0
}
"#;

#[test]
fn s2_the_generic_library_answers_on_char_at_every_level() {
    let h = Harness::new();
    h.value_row(
        "S2-LIB-CHAR",
        LIBRARY_ON_CHAR,
        "d\nc\ne\na\n2\n-1\n4\n1\n1\n1\n0\n1\n3\n-1\ne\na\nq\n",
    );
    h.assert_legs(8);
}

// two element types in one program: the monomorphiser has to keep the two bodies apart
const TWO_ELEMENT_TYPES_IN_ONE_PROGRAM: &str = r#"
needs std.result
needs std.slice
needs std.sort

fn main() -> i64 {
    let mut a: [i64;4] = [4, 2, 3, 1]
    let mut c: [char;4] = ['d', 'b', 'c', 'a']
    sort.insertion_sort(a[..])
    sort.insertion_sort(c[..])
    println(result.some_or(slice.first(a[..]), -777))
    println(result.some_or(slice.first(c[..]), 'z'))
    println(sort.binary_search(a[..], 3))
    println(sort.binary_search(c[..], 'c'))
    println(slice.index_of(a[..], 2))
    println(slice.index_of(c[..], 'b'))
    return 0
}
"#;

#[test]
fn s2_two_element_types_in_one_program_keep_their_own_answers() {
    let h = Harness::new();
    h.value_row(
        "S2-TWO-TYPES",
        TWO_ELEMENT_TYPES_IN_ONE_PROGRAM,
        "1\na\n2\n2\n1\n1\n",
    );
    h.assert_legs(8);
}

// the witness is the compile-time fence, the rc counter is not level-invariant on this repo
const NOGC_FENCE_ON_BOTH_ELEMENTS: &str = r#"
needs std.result
needs std.slice
needs std.sort

nogc fn every_entry_point(r: &[i64], w: &mut [i64], c: &[char], d: &mut [char]) -> i64 {
    let mut acc: i64 = 0
    acc = acc + slice.sum(r)
    acc = acc + result.some_or(slice.first(r), 0)
    acc = acc + result.some_or(slice.last(r), 0)
    acc = acc + result.some_or(slice.max(r), 0)
    acc = acc + result.some_or(slice.min(r), 0)
    acc = acc + slice.index_of(r, 3)
    acc = acc + result.some_or(slice.find(r, 3), 0)
    if slice.contains(r, 3) { acc = acc + 1 }
    if slice.eq(r, r) { acc = acc + 1 }
    if slice.is_sorted(r) { acc = acc + 1 }
    slice.fill(w, 2)
    slice.swap(w, 0, 3)
    slice.reverse(w)
    sort.insertion_sort(w)
    acc = acc + sort.binary_search(w, 2)
    acc = acc + slice.index_of(c, 'c')
    if slice.contains(c, 'a') { acc = acc + 1 }
    if slice.eq(c, c) { acc = acc + 1 }
    if slice.is_sorted(c) { acc = acc + 1 }
    slice.fill(d, 'q')
    slice.swap(d, 0, 2)
    slice.reverse(d)
    sort.insertion_sort(d)
    if result.is_some(slice.max(d)) { acc = acc + 1 }
    return acc
}

fn main() -> i64 {
    let a: [i64;5] = [1, 2, 3, 4, 5]
    let mut b: [i64;4] = [0, 0, 0, 0]
    let c: [char;3] = ['a', 'b', 'c']
    let mut d: [char;3] = ['x', 'y', 'z']
    println(every_entry_point(a[..], b[..], c[..], d[..]))
    return 0
}
"#;

#[test]
fn s2_the_whole_generic_library_stays_callable_from_nogc() {
    let h = Harness::new();
    h.value_row("S2-NOGC-FENCE", NOGC_FENCE_ON_BOTH_ELEMENTS, "41\n");
    h.assert_legs(8);
}

// `ord` excludes `string`, so ordering strings keeps its own monomorphic entry point
const SORT_STRINGS_SURVIVES: &str = r#"
needs std.sort

fn main() -> i64 {
    let mut s: [string;4] = ["pear", "apple", "fig", "date"]
    sort.strings(s[..])
    println(s[0])
    println(s[1])
    println(s[2])
    println(s[3])
    return 0
}
"#;

#[test]
fn s2_sort_strings_survives_because_ord_excludes_string() {
    let h = Harness::new();
    h.value_row(
        "S2-SORT-STRINGS",
        SORT_STRINGS_SURVIVES,
        "apple\ndate\nfig\npear\n",
    );
    h.assert_legs(8);
}

const INSERTION_SORT_ON_STRINGS: &str = r#"
needs std.sort

fn main() -> i64 {
    let mut s: [string;2] = ["b", "a"]
    sort.insertion_sort(s[..])
    return 0
}
"#;

#[test]
fn s2_the_generic_sort_refuses_a_string_slice() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S2-SORT-STRING-REFUSED",
        INSERTION_SORT_ON_STRINGS,
        "E0730",
        &[
            "type parameter `T` of `std.sort.insertion_sort` is bound `nogc`, but this call \
             instantiates it with `string`",
        ],
    );
}

const ORDER_ON_A_STRING: &str = r#"
fn below<T: ord>(a: T, b: T) -> bool { return a < b }

fn main() -> i64 {
    if below("a", "b") { return 1 }
    return 0
}
"#;

// the limit is on record here rather than discovered by a user: `ord` is not `string`
#[test]
fn s2_the_ord_bound_refuses_string_and_points_at_str_compare() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S2-ORD-STRING",
        ORDER_ON_A_STRING,
        "E0732",
        &[
            "is bound `ord`, but this call instantiates it with `string`, which is not `ord`",
            "`string` is not, use `str.compare`",
        ],
    );
}

const SLICE_ELEMENT_BINDS_THE_TYPE_PARAMETER: &str = r#"
fn first<T>(s: &[T]) -> T { return s[0] }

fn main() -> i64 {
    let a: [i64;3] = [7, 8, 9]
    println(first(a[..]))
    let c: [char;3] = ['p', 'q', 'r']
    println(first(c[..]))
    return 0
}
"#;

#[test]
fn s2_unification_descends_into_the_element_of_a_slice() {
    let h = Harness::new();
    h.value_row(
        "S2-SLICE-UNIFY",
        SLICE_ELEMENT_BINDS_THE_TYPE_PARAMETER,
        "7\np\n",
    );
    h.assert_legs(8);
}

// descending into the element must not unify the mutability away
const SHARED_SLICE_INTO_A_MUT_PARAMETER: &str = r#"
fn zap<T>(s: &mut [T]) { s[0] = s[1] }

fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    zap(a[..])
    return 0
}
"#;

#[test]
fn s2_a_shared_slice_still_cannot_reach_a_mut_slice_parameter() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S2-SLICE-MUT",
        SHARED_SLICE_INTO_A_MUT_PARAMETER,
        "E0416",
        &["a shared borrow `&[i64]` cannot be used where the mutable borrow `&mut [T]`"],
    );
}

const MUT_SLICE_STILL_REACHES_A_MUT_PARAMETER: &str = r#"
fn zap<T>(s: &mut [T]) { s[0] = s[1] }

fn main() -> i64 {
    let mut a: [i64;3] = [1, 2, 3]
    zap(a[..])
    println(a[0])
    return 0
}
"#;

#[test]
fn s2_a_mut_slice_still_reaches_a_mut_slice_parameter() {
    let h = Harness::new();
    h.value_row(
        "S2-SLICE-MUT-OK",
        MUT_SLICE_STILL_REACHES_A_MUT_PARAMETER,
        "2\n",
    );
    h.assert_legs(8);
}

const EQUALITY_ON_AN_UNBOUNDED_TYPE_PARAMETER: &str = r#"
fn same<T>(a: T, b: T) -> bool { return a == b }

fn main() -> i64 {
    if same(1, 1) { return 1 }
    return 0
}
"#;

#[test]
fn s2_equality_on_an_unbounded_type_parameter_names_the_missing_bound() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S2-EQ-UNBOUND",
        EQUALITY_ON_AN_UNBOUNDED_TYPE_PARAMETER,
        "E0732",
        &[
            "`==` needs the `eq` bound on type parameter `T`, and `T` carries no bound",
            "declare it as `<T: eq>`",
        ],
    );
}

const ORDER_ON_AN_UNBOUNDED_TYPE_PARAMETER: &str = r#"
fn below<T>(a: T, b: T) -> bool { return a < b }

fn main() -> i64 {
    if below(1, 2) { return 1 }
    return 0
}
"#;

#[test]
fn s2_order_on_an_unbounded_type_parameter_names_the_missing_bound() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S2-ORD-UNBOUND",
        ORDER_ON_AN_UNBOUNDED_TYPE_PARAMETER,
        "E0732",
        &[
            "`<` needs the `ord` bound on type parameter `T`, and `T` carries no bound",
            "declare it as `<T: ord>`",
        ],
    );
}

// the old refusal called `t` a struct and the operation unsupported; both were wrong
#[test]
fn s2_the_old_wrong_refusals_are_gone() {
    let h = Harness::new();
    for (id, src) in [
        ("S2-OLD-EQ", EQUALITY_ON_AN_UNBOUNDED_TYPE_PARAMETER),
        ("S2-OLD-ORD", ORDER_ON_AN_UNBOUNDED_TYPE_PARAMETER),
    ] {
        for (tag, opt) in LEVELS {
            let root = h.stage(id, tag, src);
            let rendered = match lower_file_to_air(&root, *opt) {
                Ok(_) => panic!("{id} at {tag}: MUST be rejected"),
                Err(rendered) => rendered,
            };
            for gone in [
                "is not supported for struct `T`",
                "type `T` is not one of",
                "E0304",
                "E0301",
            ] {
                assert!(
                    !rendered.contains(gone),
                    "{id} at {tag}: the old refusal {gone:?} must not survive\n{rendered}"
                );
            }
        }
    }
}

const EQ_BOUND_IS_ENOUGH_FOR_EQUALITY: &str = r#"
fn same<T: eq>(a: T, b: T) -> bool { return a == b }

fn main() -> i64 {
    if same(1, 1) { println(1) }
    if same('a', 'a') { println(2) }
    if same("x", "x") { println(3) }
    if same(true, true) { println(4) }
    if same(1.5, 1.5) { println(5) }
    return 0
}
"#;

#[test]
fn s2_the_eq_bound_answers_on_everything_the_language_compares() {
    let h = Harness::new();
    h.value_row(
        "S2-EQ-BOUND",
        EQ_BOUND_IS_ENOUGH_FOR_EQUALITY,
        "1\n2\n3\n4\n5\n",
    );
    h.assert_legs(8);
}

const ORD_BOUND_IS_ENOUGH_FOR_ORDER: &str = r#"
fn below<T: ord>(a: T, b: T) -> bool { return a < b }
fn same_or_below<T: ord>(a: T, b: T) -> bool { return a <= b and a == b or true }

fn main() -> i64 {
    if below(1, 2) { println(1) }
    if below('a', 'b') { println(2) }
    if below(1.5, 2.5) { println(3) }
    if same_or_below(1, 1) { println(4) }
    return 0
}
"#;

#[test]
fn s2_the_ord_bound_implies_eq() {
    let h = Harness::new();
    h.value_row(
        "S2-ORD-BOUND",
        ORD_BOUND_IS_ENOUGH_FOR_ORDER,
        "1\n2\n3\n4\n",
    );
    h.assert_legs(8);
}

const COMBINED_BOUNDS: &str = r#"
nogc fn ranked<T: nogc + ord>(s: &mut [T]) -> i64 {
    let mut i: i64 = 1
    while i < s.len {
        let x: T = s[i]
        let mut j: i64 = i - 1
        while j >= 0 and s[j] > x {
            s[j + 1] = s[j]
            j = j - 1
        }
        s[j + 1] = x
        i = i + 1
    }
    return s.len
}

fn main() -> i64 {
    let mut a: [i64;3] = [3, 1, 2]
    println(ranked(a[..]))
    println(a[0])
    let mut c: [char;3] = ['c', 'a', 'b']
    println(ranked(c[..]))
    println(c[0])
    return 0
}
"#;

#[test]
fn s2_bounds_combine_with_a_plus() {
    let h = Harness::new();
    h.value_row("S2-PLUS", COMBINED_BOUNDS, "3\n1\n3\na\n");
    h.assert_legs(8);
}

const AN_INVENTED_BOUND: &str = r#"
fn f<T: hash>(a: T) -> bool { return true }

fn main() -> i64 { return 0 }
"#;

#[test]
fn s2_an_invented_bound_is_refused_by_naming_the_three_that_exist() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S2-BAD-BOUND",
        AN_INVENTED_BOUND,
        "E0106",
        &["`hash` is not a bound; the bounds are `nogc`, `eq` and `ord`, combined with `+`"],
    );
}

const EQ_AND_ORD_ARE_STILL_ORDINARY_NAMES: &str = r#"
nogc fn eq(a: i64, b: i64) -> bool { return a == b }
nogc fn ord(c: char) -> bool { return c > 'a' }

fn main() -> i64 {
    let eq2: i64 = 3
    let ord2: i64 = 4
    if eq(eq2, 3) { println(1) }
    if ord('b') { println(2) }
    println(eq2 + ord2)
    return 0
}
"#;

#[test]
fn s2_eq_and_ord_did_not_become_keywords() {
    let h = Harness::new();
    h.value_row(
        "S2-NOT-KEYWORDS",
        EQ_AND_ORD_ARE_STILL_ORDINARY_NAMES,
        "1\n2\n7\n",
    );
    h.assert_legs(8);
}

const A_STRUCT_SATISFIES_NEITHER: &str = r#"
struct P { x: i64 }

fn same<T: eq>(a: T, b: T) -> bool { return a == b }

fn main() -> i64 {
    let p: P = P { x: 1 }
    let q: P = P { x: 1 }
    if same(p, q) { return 1 }
    return 0
}
"#;

#[test]
fn s2_a_struct_satisfies_no_bound() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S2-STRUCT-NOT-EQ",
        A_STRUCT_SATISFIES_NEITHER,
        "E0732",
        &["is bound `eq`, but this call instantiates it with `P`, which is not `eq`"],
    );
}

const A_VEC_SATISFIES_NEITHER: &str = r#"
fn same<T: eq>(a: T, b: T) -> bool { return a == b }

fn main() -> i64 {
    let v: Vec<i64> = Vec::new()
    let w: Vec<i64> = Vec::new()
    if same(v, w) { return 1 }
    return 0
}
"#;

#[test]
fn s2_a_vec_satisfies_no_bound() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S2-VEC-NOT-EQ",
        A_VEC_SATISFIES_NEITHER,
        "E0732",
        &["is bound `eq`, but this call instantiates it with `vec[i64]`, which is not `eq`"],
    );
}

// a bounded generic may hand its own parameter to another one carrying the same bound
const A_BOUND_PROPAGATES_BETWEEN_GENERICS: &str = r#"
nogc fn idx<T: eq>(s: &[T], x: T) -> i64 {
    for i in 0..s.len {
        if s[i] == x { return i }
    }
    return -1
}

nogc fn has<T: eq>(s: &[T], x: T) -> bool { return idx(s, x) >= 0 }

fn main() -> i64 {
    let a: [i64;3] = [1, 2, 3]
    if has(a[..], 2) { println(1) }
    let c: [char;3] = ['a', 'b', 'c']
    if has(c[..], 'b') { println(2) }
    return 0
}
"#;

#[test]
fn s2_a_bound_propagates_from_one_generic_to_the_next() {
    let h = Harness::new();
    h.value_row(
        "S2-PROPAGATE",
        A_BOUND_PROPAGATES_BETWEEN_GENERICS,
        "1\n2\n",
    );
    h.assert_legs(8);
}

const A_WEAKER_BOUND_DOES_NOT_REACH_A_STRONGER_ONE: &str = r#"
nogc fn below<T: ord>(a: T, b: T) -> bool { return a < b }

nogc fn same<T: eq>(a: T, b: T) -> bool { return below(a, b) }

fn main() -> i64 {
    if same(1, 2) { return 1 }
    return 0
}
"#;

#[test]
fn s2_an_eq_parameter_cannot_satisfy_an_ord_bound() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S2-EQ-NOT-ORD",
        A_WEAKER_BOUND_DOES_NOT_REACH_A_STRONGER_ONE,
        "E0732",
        &["is bound `ord`, but this call instantiates it with `T`, which is not `ord`"],
    );
}

const SCALARS: &[(&str, &str, &str)] = &[
    ("i8", "1", "2"),
    ("i16", "1", "2"),
    ("i32", "1", "2"),
    ("i64", "1", "2"),
    ("u8", "1", "2"),
    ("u16", "1", "2"),
    ("u32", "1", "2"),
    ("u64", "1", "2"),
    ("f32", "1.5", "2.5"),
    ("f64", "1.5", "2.5"),
    ("bool", "true", "false"),
    ("char", "'a'", "'b'"),
    ("string", "\"a\"", "\"b\""),
];

#[test]
fn s2_the_eq_bound_holds_of_exactly_what_the_equality_operator_answers() {
    let h = Harness::new();
    let mut disagreements = Vec::new();
    for (ty, lo, hi) in SCALARS {
        let direct = h.accepts_once(
            &format!("S2-DERIVE-EQ-DIRECT-{ty}"),
            &format!(
                "fn main() -> i64 {{\n    let a: {ty} = {lo}\n    let b: {ty} = {hi}\n    \
                 if a == b {{ return 1 }}\n    return 0\n}}\n"
            ),
        );
        let bounded = h.accepts_once(
            &format!("S2-DERIVE-EQ-BOUND-{ty}"),
            &format!(
                "fn same<T: eq>(a: T, b: T) -> bool {{ return a == b }}\n\
                 fn main() -> i64 {{\n    let a: {ty} = {lo}\n    let b: {ty} = {hi}\n    \
                 if same(a, b) {{ return 1 }}\n    return 0\n}}\n"
            ),
        );
        if direct != bounded {
            disagreements.push(format!(
                "{ty}: `==` accepts {direct}, `T: eq` accepts {bounded}"
            ));
        }
    }
    assert!(
        disagreements.is_empty(),
        "the `eq` bound must hold of exactly the types `==` answers on, or the bound has drifted \
         away from the operator it stands for:\n{}",
        disagreements.join("\n")
    );
}

#[test]
fn s2_the_ord_bound_holds_of_exactly_what_the_order_operator_answers() {
    let h = Harness::new();
    let mut disagreements = Vec::new();
    let mut ordered = Vec::new();
    for (ty, lo, hi) in SCALARS {
        let direct = h.accepts_once(
            &format!("S2-DERIVE-ORD-DIRECT-{ty}"),
            &format!(
                "fn main() -> i64 {{\n    let a: {ty} = {lo}\n    let b: {ty} = {hi}\n    \
                 if a < b {{ return 1 }}\n    return 0\n}}\n"
            ),
        );
        let bounded = h.accepts_once(
            &format!("S2-DERIVE-ORD-BOUND-{ty}"),
            &format!(
                "fn below<T: ord>(a: T, b: T) -> bool {{ return a < b }}\n\
                 fn main() -> i64 {{\n    let a: {ty} = {lo}\n    let b: {ty} = {hi}\n    \
                 if below(a, b) {{ return 1 }}\n    return 0\n}}\n"
            ),
        );
        if direct != bounded {
            disagreements.push(format!(
                "{ty}: `<` accepts {direct}, `T: ord` accepts {bounded}"
            ));
        }
        if bounded {
            ordered.push(*ty);
        }
    }
    assert!(
        disagreements.is_empty(),
        "the `ord` bound must hold of exactly the types `<` answers on:\n{}",
        disagreements.join("\n")
    );
    assert_eq!(
        ordered,
        vec![
            "i8", "i16", "i32", "i64", "u8", "u16", "u32", "u64", "f32", "f64", "char"
        ],
        "`ord` must be the integers, the floats and `char`, and `string` must not be among them"
    );
}

const A_MANAGED_ELEMENT_IN_A_NOGC_GENERIC: &str = r#"
needs std.slice

fn main() -> i64 {
    let s: [string;2] = ["a", "b"]
    if slice.contains(s[..], "a") { return 1 }
    return 0
}
"#;

#[test]
fn s2_a_string_element_is_refused_by_the_nogc_bound_the_library_carries() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S2-NOGC-STRING-ELEM",
        A_MANAGED_ELEMENT_IN_A_NOGC_GENERIC,
        "E0730",
        &["is bound `nogc`, but this call instantiates it with `string`"],
    );
}

// a generic that is not `nogc` takes a managed element, so the bound is what excludes it
const A_MANAGED_ELEMENT_OUTSIDE_NOGC: &str = r#"
fn idx<T: eq>(s: &[T], x: T) -> i64 {
    for i in 0..s.len {
        if s[i] == x { return i }
    }
    return -1
}

fn main() -> i64 {
    let s: [string;3] = ["a", "b", "c"]
    println(idx(s[..], "b"))
    return 0
}
"#;

#[test]
fn s2_a_string_element_is_fine_once_the_nogc_bound_is_not_there() {
    let h = Harness::new();
    h.value_row("S2-STRING-ELEM-OK", A_MANAGED_ELEMENT_OUTSIDE_NOGC, "1\n");
    h.assert_legs(8);
}

// every `std.slice` and `std.sort` entry point must still be reachable from a `nogc fn`
#[test]
fn s2_every_library_entry_point_is_still_declared_nogc() {
    let root = repo_root().join("std");
    for module in ["slice.aelys", "sort.aelys"] {
        let text = fs::read_to_string(root.join(module))
            .unwrap_or_else(|e| panic!("std/{module} must be readable: {e}"));
        let mut entry_points = 0usize;
        for line in text.lines() {
            let line = line.trim_start();
            if !line.starts_with("pub ") {
                continue;
            }
            entry_points += 1;
            assert!(
                line.starts_with("pub nogc fn "),
                "std/{module}: `{line}` is public and not `nogc`, so the nogc-callable surface \
                 shrank"
            );
        }
        assert!(
            entry_points >= 3,
            "std/{module}: only {entry_points} public entry points were seen, which reads as a \
             broken scan rather than a small module"
        );
    }
}

#[test]
fn s2_the_library_is_generic_and_sort_strings_is_the_named_exception() {
    let text = fs::read_to_string(repo_root().join("std").join("slice.aelys"))
        .expect("std/slice.aelys must be readable");
    for generic in [
        "fn first<T>",
        "fn last<T>",
        "fn max<T: ord>",
        "fn min<T: ord>",
        "fn index_of<T: eq>",
        "fn find<T: eq>",
        "fn contains<T: eq>",
        "fn eq<T: eq>",
        "fn is_sorted<T: ord>",
        "fn fill<T>",
        "fn swap<T>",
        "fn reverse<T>",
    ] {
        assert!(
            text.contains(generic),
            "std/slice.aelys must declare `{generic}`; a signature that went back to `i64` \
             undoes this stage silently\n{text}"
        );
    }
    assert!(
        text.contains("fn sum(s: &[i64]) -> i64"),
        "`sum` needs arithmetic on the element and there is no bound for that, so it stays `i64` \
         until one exists\n{text}"
    );

    let text = fs::read_to_string(repo_root().join("std").join("sort.aelys"))
        .expect("std/sort.aelys must be readable");
    for generic in ["fn insertion_sort<T: ord>", "fn binary_search<T: ord>"] {
        assert!(
            text.contains(generic),
            "std/sort.aelys must declare `{generic}`\n{text}"
        );
    }
    assert!(
        text.contains("fn strings(s: &mut [string])"),
        "`ord` excludes `string`, so `sort.strings` is the entry point that orders them and it \
         must survive\n{text}"
    );
}

#[test]
fn s2_the_bound_refusal_explains_itself() {
    let rows = aelys_common::diagnostic::registry::all_codes();
    for (code, wanted) in [
        (
            "E0732",
            vec![
                "`==` and `!=` on a type parameter need `eq`",
                "fn lt<T: ord>(a: T, b: T) -> bool { return a < b }",
                "`string` is\ndeliberately not `ord`",
            ],
        ),
        (
            "E0106",
            vec![
                "fn sort<T: nogc + ord>(s: &mut [T]) { ... }",
                "`eq` and `ord` are read as ordinary names here, not keywords",
            ],
        ),
    ] {
        let row = rows
            .iter()
            .find(|r| r.code == code)
            .unwrap_or_else(|| panic!("{code} must have a registry row"));
        for want in wanted {
            assert!(
                row.explanation.contains(want),
                "`aelys --explain {code}` must say {want:?}; a refusal the user meets and the \
                 explanation does not mention reads as a compiler bug\n{}",
                row.explanation
            );
        }
    }
}

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

const MODULES: &[&str] = &["io.aelys", "result.aelys", "str.aelys"];

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
    ) -> Option<PathBuf> {
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
                    "{id} at {tag}/{alloc_name}: stdout MUST be {stdout:?}\nstderr:\n{seen_err}"
                );
                if let Some(want) = stats {
                    let got = parse_stats(&seen_err).unwrap_or_else(|| {
                        panic!(
                            "{id} at {tag}/{alloc_name}: no [rc] stats line\nstderr:\n{seen_err}"
                        )
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
        self.row(id, src, stdout, Some((stdout.lines().count() as i64, 0)));
    }

    fn rejects_at_every_level(&self, id: &str, src: &str, code: &str, says: &str) {
        let root = self.stage(id, "reject", src);
        let mut seen: Vec<(&str, String)> = Vec::new();
        for (tag, opt) in LEVELS {
            let rendered = match lower_file_to_air(&root, *opt) {
                Ok(_) => panic!("{id} at {tag}: MUST be rejected\n{src}"),
                Err(rendered) => rendered,
            };
            self.legs.set(self.legs.get() + 1);
            assert!(
                rendered.contains(&format!("[{code}]")),
                "{id} at {tag}: the rejection MUST be {code}\n{src}\nrendered:\n{rendered}"
            );
            assert!(
                rendered.contains(says),
                "{id} at {tag}: the rejection MUST say {says:?}\n{src}\nrendered:\n{rendered}"
            );
            seen.push((tag, rendered));
        }
        let (first_tag, first) = &seen[0];
        for (tag, rendered) in &seen[1..] {
            assert_eq!(
                rendered, first,
                "{id}: the refusal at {tag} differs from {first_tag}; a rule that reads the \
                 optimizer is not a rule"
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
             is a confident zero"
        );
    }
}

const NO_PLACE_BYTES: &str = "[no-place] the receiver of `.bytes` denotes no storage, so it has \
                              no address. bind it to a name first and use that binding";
const NO_PLACE_REF: &str = "[no-place] the operand of `&` denotes no storage, so it has no \
                            address. bind it to a name first and use that binding";

const RC_PREAMBLE: &str = r#"
struct Inner { n: string }
struct S { n: string, i: Inner, pad: i64 }

fn mkrc() -> Rc<S> {
    return Rc::new(S { n: "hello", i: Inner { n: "hello" }, pad: 7 })
}

fn mkrcstr() -> Rc<string> { return Rc::new("hello") }
"#;

fn rc_escape(body: &str) -> String {
    format!("{RC_PREAMBLE}\nfn main() -> i64 {{\n{body}\n    return 0\n}}\n")
}

#[test]
fn s13_1_a_projection_through_an_unbound_rc_temporary_is_not_a_place() {
    let h = Harness::new();
    for (id, body) in [
        (
            "S1.3-1a",
            "    let v: &[u8] = mkrc().n.bytes\n    println(v[0] as i64)",
        ),
        (
            "S1.3-1b",
            "    let v: &[u8] = Rc::get(mkrc()).n.bytes\n    println(v[0] as i64)",
        ),
        (
            "S1.3-1c",
            "    let v: &[u8] = mkrc().i.n.bytes\n    println(v[0] as i64)",
        ),
        (
            "S1.3-1d",
            "    let v: &[u8] = Rc::get(mkrcstr()).bytes\n    println(v[0] as i64)",
        ),
    ] {
        h.rejects_at_every_level(id, &rc_escape(body), "E0421", NO_PLACE_BYTES);
    }
    for (id, body) in [
        (
            "S1.3-1e",
            "    let q: &string = &mkrc().n\n    println((*q).len)",
        ),
        (
            "S1.3-1f",
            "    let q: &string = &Rc::get(mkrc()).n\n    println((*q).len)",
        ),
    ] {
        h.rejects_at_every_level(id, &rc_escape(body), "E0421", NO_PLACE_REF);
    }
    h.assert_legs(24);
}

const S13_STRUCT_TEMPORARY_CONTROL: &str = r#"
struct S { n: string, pad: i64 }

fn mkstruct() -> S { return S { n: "hello", pad: 7 } }

fn main() -> i64 {
    let v: &[u8] = mkstruct().n.bytes
    println(v[0] as i64)
    return 0
}
"#;

#[test]
fn s13_2_the_struct_temporary_control_is_refused_by_the_same_rule() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S1.3-2",
        S13_STRUCT_TEMPORARY_CONTROL,
        "E0421",
        NO_PLACE_BYTES,
    );
    h.assert_legs(4);
}

const S13_BOUND_RC_STAYS_A_PLACE: &str = r#"
struct S { n: string, pad: i64 }

fn churn(n: i64) -> i64 {
    let mut acc: i64 = 0
    let mut i: i64 = 0
    while i < n {
        let t: Rc<S> = Rc::new(S { n: "0123456789abcdef", pad: i })
        acc = acc + Rc::get(t).pad
        i = i + 1
    }
    return acc
}

fn main() -> i64 {
    let r: Rc<S> = Rc::new(S { n: "hello", pad: 7 })
    let v: &[u8] = r.n.bytes
    let q: &string = &r.n
    println(churn(300))
    println(v[0] as i64)
    println(v.len)
    println((*q).len)
    println(Rc::get(r).pad)
    Rc::get(r).pad = 9
    println(Rc::get(r).pad)
    return 0
}
"#;

const S13_BOUND_RC_STRING_STAYS_A_PLACE: &str = r#"
struct S { n: string, pad: i64 }

fn churn(n: i64) -> i64 {
    let mut acc: i64 = 0
    let mut i: i64 = 0
    while i < n {
        let t: Rc<S> = Rc::new(S { n: "0123456789abcdef", pad: i })
        acc = acc + Rc::get(t).pad
        i = i + 1
    }
    return acc
}

fn main() -> i64 {
    let r: Rc<string> = Rc::new("hello")
    let v: &[u8] = Rc::get(r).bytes
    println(churn(300))
    println(v[0] as i64)
    println(v.len)
    return 0
}
"#;

#[test]
fn s13_3_a_bound_rc_projection_still_compiles_and_runs() {
    let h = Harness::new();
    h.value_row(
        "S1.3-3a",
        S13_BOUND_RC_STAYS_A_PLACE,
        "44850\n104\n5\n5\n7\n9\n",
    );
    h.value_row(
        "S1.3-3b",
        S13_BOUND_RC_STRING_STAYS_A_PLACE,
        "44850\n104\n5\n",
    );
    h.assert_legs(16);
}

const S13_GENERIC_INDEX: &str = r#"
fn at<T>(x: T, i: i64) -> string { return x[i] }

fn main() -> i64 {
    println(at("eab", 1))
    return 0
}
"#;

const S13_GENERIC_INDEX_SAYS: &str = "a value of the generic type parameter `T` cannot be \
                                      indexed; the element type is not known until `T` is bound, \
                                      and a type parameter carries no bound that would supply one";

#[test]
fn s13_4_a_generic_index_is_refused_by_the_index_rule_that_owns_it() {
    let h = Harness::new();
    h.rejects_at_every_level("S1.3-4", S13_GENERIC_INDEX, "E0304", S13_GENERIC_INDEX_SAYS);
    h.assert_legs(4);
}

const S13_GENERIC_CONTAINER_INDEX_STILL_WORKS: &str = r#"
fn at<T>(v: Vec<T>, i: i64) -> T { return v[i] }

fn set<T>(x: T, i: i64) -> i64 {
    let mut y: T = x
    y[i] = 5
    return 0
}

fn main() -> i64 {
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 7)
    println(at(v, 0))
    let a: [i64; 3] = [1, 2, 3]
    println(set(a, 1))
    return 0
}
"#;

#[test]
fn s13_5_indexing_a_container_of_a_type_parameter_still_compiles_and_runs() {
    let h = Harness::new();
    h.value_row("S1.3-5", S13_GENERIC_CONTAINER_INDEX_STILL_WORKS, "7\n0\n");
    h.assert_legs(8);
}

const S13_IS_SPACE_TAKES_A_CHAR: &str = r#"
needs std.str

nogc fn count_spaces(s: string) -> i64 {
    let mut n: i64 = 0
    for c in s {
        if str.is_space(c) { n = n + 1 }
    }
    return n
}

fn main() -> i64 {
    println(count_spaces("a b\tc\nd\re"))
    if str.is_space(' ') { println(1) } else { println(0) }
    if str.is_space('x') { println(1) } else { println(0) }
    return 0
}
"#;

#[test]
fn s13_6_is_space_is_fed_by_the_loop_the_library_hands_you() {
    let h = Harness::new();
    h.nogc_row("S1.3-6", S13_IS_SPACE_TAKES_A_CHAR, "4\n1\n0\n");
    h.assert_legs(8);
}

const S13_STRING_INDEX_STAYS_REFUSED: &str = r#"
fn main() -> i64 {
    let s: string = "eab"
    println(s[1])
    return 0
}
"#;

const S13_CHAR_READING_SURFACE: &str = r#"
needs std.result
needs std.str

fn main() -> i64 {
    let s: string = "éab"
    for c in s {
        println(c)
        println(c as i64)
    }
    println(result.some_or(str.char_at(s, 0), '?'))
    println(str.char_count(s))
    println(s.len)
    return 0
}
"#;

#[test]
fn s13_7_the_string_index_producer_is_gone_at_the_surface_and_the_char_reader_remains() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S1.3-7a",
        S13_STRING_INDEX_STAYS_REFUSED,
        "E0304",
        "a `string` cannot be indexed by integer",
    );
    h.value_row(
        "S1.3-7b",
        S13_CHAR_READING_SURFACE,
        "é\n233\na\n97\nb\n98\né\n3\n4\n",
    );
    h.assert_legs(12);
}

const S13_ANNOTATION_DIRECTION: &str = r#"
fn main() -> i64 {
    let c: char = 97
    return 0
}
"#;

const S13_ASSIGNMENT_DIRECTION: &str = r#"
fn main() -> i64 {
    let mut s: string = "a"
    s = 97
    return 0
}
"#;

const S13_FIELD_DIRECTION: &str = r#"
struct S { c: char }

fn main() -> i64 {
    let s = S { c: 97 }
    return 0
}
"#;

const S13_CONDITION_DIRECTION: &str = r#"
fn main() -> i64 {
    if 1 { return 1 }
    return 0
}
"#;

const S13_ARGUMENT_DIRECTION: &str = r#"
fn f(c: char) -> i64 { return c as i64 }

fn main() -> i64 {
    return f(97)
}
"#;

const S13_RETURN_DIRECTION: &str = r#"
fn f() -> i64 { return "hi" }

fn main() -> i64 { return f() }
"#;

#[test]
fn s13_8_a_mismatch_names_what_the_language_asked_for_first() {
    let h = Harness::new();
    for (id, src, says) in [
        (
            "S1.3-8a",
            S13_ANNOTATION_DIRECTION,
            "expected `char`, found `i64`",
        ),
        (
            "S1.3-8b",
            S13_ASSIGNMENT_DIRECTION,
            "expected `string`, found `i64`",
        ),
        (
            "S1.3-8c",
            S13_FIELD_DIRECTION,
            "expected `char`, found `i64`",
        ),
        (
            "S1.3-8d",
            S13_CONDITION_DIRECTION,
            "expected `bool`, found `i64`",
        ),
        (
            "S1.3-8e",
            S13_ARGUMENT_DIRECTION,
            "expected `char`, found `i64`",
        ),
        (
            "S1.3-8f",
            S13_RETURN_DIRECTION,
            "expected `i64`, found `string`",
        ),
    ] {
        h.rejects_at_every_level(id, src, "E0301", says);
    }
    h.assert_legs(24);
}

// a title stating nothing allocates still pins a non-zero count, since printing moved from the raw counter

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

    fn value_row(&self, id: &str, src: &str, stdout: &str, stats: (i64, i64)) {
        for (tag, opt) in LEVELS {
            let root = self.stage(id, tag, src);
            let Some(exe) = self.compile_at(id, tag, &root, *opt, RuntimeVariant::Rc) else {
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
                let got = parse_stats(&seen_err).unwrap_or_else(|| {
                    panic!("{id} at {tag}/{alloc_name}: no [rc] stats line\nstderr:\n{seen_err}")
                });
                assert_eq!(
                    got, stats,
                    "{id} at {tag}/{alloc_name}: MUST be allocs={} frees={}",
                    stats.0, stats.1
                );
            }
        }
    }

    fn panic_row(&self, id: &str, src: &str, says: &str) {
        for (tag, opt) in LEVELS {
            let root = self.stage(id, tag, src);
            let Some(exe) = self.compile_at(id, tag, &root, *opt, RuntimeVariant::Rc) else {
                assert!(
                    linker_skip_declared(),
                    "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped panic \
                     row carries no runtime evidence at all"
                );
                return;
            };
            for (alloc_name, alloc) in ALLOCATORS {
                let mut cmd = Command::new(&exe);
                if let Some(a) = alloc {
                    cmd.env("AELYS_ALLOC", a);
                }
                let out = cmd.output().expect("run compiled exe");
                let seen_err = String::from_utf8_lossy(&out.stderr).into_owned();
                self.legs.set(self.legs.get() + 1);
                assert_eq!(
                    exit_code(&out.status),
                    134,
                    "{id} at {tag}/{alloc_name} MUST abort\nstderr:\n{seen_err}"
                );
                assert!(
                    seen_err.contains(says),
                    "{id} at {tag}/{alloc_name}: the panic MUST say {says:?}\nstderr:\n{seen_err}"
                );
            }
        }
    }

    fn rejects(&self, id: &str, src: &str, code: &str, says: &str) {
        let root = self.stage(id, "reject", src);
        let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
            Ok(_) => panic!("{id}: MUST be rejected\n{src}"),
            Err(rendered) => rendered,
        };
        self.legs.set(self.legs.get() + 1);
        assert!(
            rendered.contains(&format!("[{code}]")),
            "{id}: the rejection MUST be {code}\n{src}\nrendered:\n{rendered}"
        );
        assert!(
            rendered.contains(says),
            "{id}: the rejection MUST say {says:?}\n{src}\nrendered:\n{rendered}"
        );
    }

    // the compiler's own advice must not hand back the escaping view s[i] was removed for
    fn refuses_without_saying(&self, id: &str, src: &str, forbidden: &str) {
        let root = self.stage(id, "reject-advice", src);
        let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
            Ok(_) => panic!("{id}: MUST be rejected\n{src}"),
            Err(rendered) => rendered,
        };
        self.legs.set(self.legs.get() + 1);
        assert!(
            !rendered.contains(forbidden),
            "{id}: the refusal MUST NOT say {forbidden:?}\n{src}\nrendered:\n{rendered}"
        );
    }

    // leak never frees, so its answer is right by construction and any divergence is an early free
    fn differential_row(&self, id: &str, src: &str, stdout: &str, counted: (i64, i64)) {
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

const SB1_BYTES_OF_A_MULTIBYTE_STRING: &str = r#"
fn main() -> i64 {
    let s: string = "éab"
    println(s.len)
    println(s.bytes.len)
    let b = s.bytes
    println(b[0])
    println(b[1])
    println(b[2])
    println(b[3])
    return 0
}
"#;

#[test]
fn sb1_bytes_answers_the_utf8_bytes_and_agrees_with_len() {
    let h = Harness::new();
    h.value_row(
        "SB-1",
        SB1_BYTES_OF_A_MULTIBYTE_STRING,
        "4\n4\n195\n169\n97\n98\n",
        (6, 0),
    );
    h.assert_legs(6);
}

const SB2_ITERATION: &str = r#"
fn main() -> i64 {
    let s: string = "éléphant"
    let b = s.bytes
    let mut sum: i64 = 0
    for i in 0..b.len {
        sum = sum + (b[i] as i64)
    }
    println(b.len)
    println(sum)
    return 0
}
"#;

#[test]
fn sb2_a_byte_view_of_a_multibyte_string_iterates() {
    let h = Harness::new();
    h.value_row("SB-2", SB2_ITERATION, "10\n1375\n", (2, 0));
    h.assert_legs(6);
}

const SB3_NOGC_BYTES: &str = r#"
nogc fn first_byte(s: string) -> i64 {
    let b = s.bytes
    if b.len == 0 { return -1 }
    return b[0] as i64
}

fn main() -> i64 {
    let s: string = "éab"
    println(first_byte(s))
    return 0
}
"#;

#[test]
fn sb3_a_byte_view_allocates_nothing_so_it_is_nogc() {
    let h = Harness::new();
    h.value_row("SB-3", SB3_NOGC_BYTES, "195\n", (1, 0));
    h.assert_legs(6);
}

const SB4_SUBSTRING_BYTES: &str = r#"
fn main() -> i64 {
    let s: string = "éléphant"
    let head = string::substring_bytes(s, 0, 5)
    let tail = string::substring_bytes(s, 5, 10)
    println(head)
    println(tail)
    println(head.len)
    return 0
}
"#;

#[test]
fn sb4_substring_bytes_cuts_a_multibyte_string_on_its_boundaries() {
    let h = Harness::new();
    h.value_row("SB-4", SB4_SUBSTRING_BYTES, "élé\nphant\n5\n", (3, 2));
    h.assert_legs(6);
}

const SB5_SUBSTRING_CONCAT: &str = r#"
fn main() -> i64 {
    let s: string = "hello world"
    let mut t: string = ""
    let mut i: i64 = 0
    for c in s {
        if i < 5 { t = t + string::from_char(c) }
        i = i + 1
    }
    println(t)
    return 0
}
"#;

const SB5_SUBSTRING_LIBRARY: &str = r#"
needs std.str

fn main() -> i64 {
    let s: string = "hello world"
    let t = str.substring(s, 0, 5)
    println(t)
    return 0
}
"#;

const SB5_SUBSTRING_BYTES: &str = r#"
fn main() -> i64 {
    let s: string = "hello world"
    let t = string::substring_bytes(s, 0, 5)
    println(t)
    return 0
}
"#;

// what still leaks is the per-character temporary and the value a call hands back, never the binding
#[test]
fn sb5_a_bound_builtin_substring_is_released_and_the_library_call_result_is_not() {
    let h = Harness::new();
    h.value_row("SB-5-concat", SB5_SUBSTRING_CONCAT, "hello\n", (10, 5));
    h.value_row("SB-5-built", SB5_SUBSTRING_BYTES, "hello\n", (1, 1));
    h.value_row("SB-5-library", SB5_SUBSTRING_LIBRARY, "hello\n", (1, 0));
    h.assert_legs(18);
}

const SB6_SPLIT_A_CHARACTER: &str = r#"
fn main() -> i64 {
    let s: string = "éab"
    let t = string::substring_bytes(s, 0, 1)
    println(t)
    return 0
}
"#;

#[test]
fn sb6_a_range_that_splits_a_character_panics_at_the_constructor() {
    let h = Harness::new();
    h.panic_row(
        "SB-6",
        SB6_SPLIT_A_CHARACTER,
        "string::substring_bytes: byte 1 is not a character boundary; it holds 0xa9, a utf-8 \
         continuation byte, so the range 0..1 would cut a character in half",
    );
    h.assert_legs(6);
}

const SB7_RANGE_OUT_OF_BOUNDS: &str = r#"
fn main() -> i64 {
    let s: string = "éab"
    let t = string::substring_bytes(s, 0, 9)
    println(t)
    return 0
}
"#;

#[test]
fn sb7_a_range_outside_the_string_panics_at_the_constructor() {
    let h = Harness::new();
    h.panic_row(
        "SB-7",
        SB7_RANGE_OUT_OF_BOUNDS,
        "string::substring_bytes: the byte range 0..9 is not inside a string of 4 bytes",
    );
    h.assert_legs(6);
}

const SB8_INDEX_REFUSED: &str = r#"
fn main() -> i64 {
    let s: string = "éab"
    println(s[3])
    return 0
}
"#;

const SB8_CHARACTERS_AND_BYTES: &str = r#"
needs std.str

fn main() -> i64 {
    let s: string = "éab"
    println(s.bytes.len)
    println(str.char_count(s))
    return 0
}
"#;

// the count that used to live in the out of bounds panic now lives in the refusal and in the pair
#[test]
fn sb8_the_refusal_separates_the_character_count_from_the_byte_length() {
    let h = Harness::new();
    h.rejects(
        "SB-8",
        SB8_INDEX_REFUSED,
        "E0304",
        "`.len` counts bytes while `s[i]` counted characters",
    );
    h.refuses_without_saying("SB-8", SB8_INDEX_REFUSED, ".bytes[");
    h.value_row("SB-8-counts", SB8_CHARACTERS_AND_BYTES, "4\n3\n", (2, 0));
    h.assert_legs(8);
}

const SB9_PREDICATES: &str = r#"
needs std.str

fn main() -> i64 {
    let s: string = "éab"
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < s.len {
        let k: i64 = str.char_len(s, i)
        println(str.is_char_boundary(s, i))
        println(k)
        i = i + k
        n = n + 1
    }
    println(str.is_char_boundary(s, 1))
    println(str.char_len(s, 1))
    println(n)
    println(s.len)
    return 0
}
"#;

#[test]
fn sb9_the_two_predicates_walk_a_multibyte_string_without_panicking() {
    let h = Harness::new();
    h.value_row(
        "SB-9",
        SB9_PREDICATES,
        "true\n2\ntrue\n1\ntrue\n1\nfalse\n-1\n3\n4\n",
        (6, 0),
    );
    h.assert_legs(6);
}

const SB10_WRITE_THROUGH_BYTES: &str = r#"
fn main() -> i64 {
    let s: string = "éab"
    let b = s.bytes
    b[0] = 65
    return 0
}
"#;

const SB11_BYTES_OUTLIVES_ITS_STRING: &str = r#"
fn main() -> i64 {
    let base: string = "éab"
    let mut r = base.bytes
    {
        let inner: string = "ünz"
        r = inner.bytes
    }
    return r[0] as i64
}
"#;

const SB12_NO_CHARACTER_COUNT: &str = r#"
fn main() -> i64 {
    let s: string = "éab"
    println(s.chars)
    return 0
}
"#;

const SB13_INDEX_ASSIGN_ON_A_STRING: &str = r#"
fn main() -> i64 {
    let mut s: string = "éab"
    s[0] = "x"
    return 0
}
"#;

const SB14_SUBSTRING_BYTES_IN_A_NOGC_FN: &str = r#"
nogc fn cut(s: string) -> string {
    return string::substring_bytes(s, 0, 2)
}

fn main() -> i64 {
    let s: string = "éab"
    println(cut(s))
    return 0
}
"#;

#[test]
fn sb10_the_fences_around_the_byte_view_all_still_fire() {
    let h = Harness::new();
    h.rejects(
        "SB-10",
        SB10_WRITE_THROUGH_BYTES,
        "E0422",
        "an indexed assignment writes through a shared `&[u8]`",
    );
    h.rejects(
        "SB-11",
        SB11_BYTES_OUTLIVES_ITS_STRING,
        "E0722",
        "`inner` does not live long enough",
    );
    h.rejects(
        "SB-12",
        SB12_NO_CHARACTER_COUNT,
        "E0304",
        "unknown field 'chars' on Str; supported: 'len' (the byte length) and 'bytes' (a shared \
         `&[u8]` view); there is no character count",
    );
    h.rejects(
        "SB-13",
        SB13_INDEX_ASSIGN_ON_A_STRING,
        "E0304",
        "a `string` cannot be written through `s[i]`",
    );
    h.rejects(
        "SB-14",
        SB14_SUBSTRING_BYTES_IN_A_NOGC_FN,
        "E0727",
        "`cut -> string::substring_bytes`",
    );
    h.assert_legs(5);
}

const SB15_A_SHARED_BINDING_THAT_ESCAPES: &str = r#"
needs std.str

fn build(src: string) -> Vec<string> {
    let mut out = Vec::new()
    let a: string = string::substring_bytes(src, 0, 3)
    let b: string = a
    Vec::push(out, b)
    return out
}

fn main() -> i64 {
    let v: Vec<string> = build("abcdefgh")
    let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 3)
    println(poison)
    println(v[0])
    return 0
}
"#;

#[test]
fn sb11_a_shared_binding_that_escapes_answers_what_the_leak_runtime_answers() {
    let h = Harness::new();
    h.differential_row(
        "SB-15",
        SB15_A_SHARED_BINDING_THAT_ESCAPES,
        "ZZZ\nabc\n",
        (3, 2),
    );
    h.assert_legs(18);
}

const SB16_A_REASSIGNED_SLOT_THAT_ESCAPES: &str = r#"
needs std.str

fn build(src: string) -> Vec<string> {
    let mut out = Vec::new()
    let mut a: string = string::substring_bytes(src, 0, 3)
    let mut b: string = string::substring_bytes(src, 3, 6)
    a = b
    Vec::push(out, b)
    println(a)
    return out
}

fn main() -> i64 {
    let v: Vec<string> = build("abcdefgh")
    let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 3)
    println(poison)
    println(v[0])
    return 0
}
"#;

const SB17_A_SLOT_REASSIGNED_TO_ITSELF: &str = r#"
needs std.str

fn main() -> i64 {
    let mut s: string = string::substring_bytes("abcdefgh", 0, 3)
    s = s
    let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 3)
    println(poison)
    println(s)
    return 0
}
"#;

#[test]
fn sb12_a_reassigned_string_answers_what_the_leak_runtime_answers() {
    let h = Harness::new();
    h.differential_row(
        "SB-16",
        SB16_A_REASSIGNED_SLOT_THAT_ESCAPES,
        "def\nZZZ\ndef\n",
        (4, 3),
    );
    h.differential_row(
        "SB-17",
        SB17_A_SLOT_REASSIGNED_TO_ITSELF,
        "ZZZ\nabc\n",
        (2, 1),
    );
    h.assert_legs(36);
}

const SB18_TWO_SLOTS_REASSIGNED_FROM_ONE_SOURCE: &str = r#"
needs std.str

fn build(src: string) -> Vec<string> {
    let mut out = Vec::new()
    let b: string = string::substring_bytes(src, 2, 5)
    Vec::push(out, b)
    {
        let mut a1: string = string::substring_bytes(src, 0, 1)
        a1 = b
        println(a1)
    }
    let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 3)
    println(poison)
    {
        let mut a2: string = string::substring_bytes(src, 1, 2)
        a2 = b
        println(a2)
    }
    let second: string = string::substring_bytes("WWWWWWWW", 0, 3)
    println(second)
    println(poison)
    return out
}

fn main() -> i64 {
    let v: Vec<string> = build("abcdefgh")
    println(v[0])
    println("survived")
    return 0
}
"#;

#[test]
fn sb13_two_reassignments_from_one_source_answer_what_the_leak_runtime_answers() {
    let h = Harness::new();
    h.differential_row(
        "SB-18",
        SB18_TWO_SLOTS_REASSIGNED_FROM_ONE_SOURCE,
        "cde\nZZZ\ncde\nWWW\nZZZ\ncde\nsurvived\n",
        (6, 5),
    );
    h.assert_legs(18);
}

// the release a reassignment emits must fall exactly when something took that value first
const SB19_A_SLOT_THAT_ESCAPES_BEFORE_IT_IS_REASSIGNED: &str = r#"
needs std.str

fn build(src: string) -> Vec<string> {
    let mut out = Vec::new()
    let mut a: string = string::substring_bytes(src, 0, 3)
    Vec::push(out, a)
    a = string::substring_bytes(src, 3, 6)
    println(a)
    return out
}

fn main() -> i64 {
    let v: Vec<string> = build("abcdefgh")
    let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 3)
    println(poison)
    println(v[0])
    return 0
}
"#;

#[test]
fn sb14_a_slot_that_escapes_before_it_is_reassigned_answers_what_the_leak_runtime_answers() {
    let h = Harness::new();
    h.differential_row(
        "SB-19",
        SB19_A_SLOT_THAT_ESCAPES_BEFORE_IT_IS_REASSIGNED,
        "def\nZZZ\nabc\n",
        (4, 2),
    );
    h.assert_legs(18);
}

const SB20_A_SLOT_REASSIGNED_IN_A_LOOP_THAT_ESCAPES_AFTER_IT: &str = r#"
needs std.str

fn touch(s: string) -> i64 {
    return s.len
}

fn main() -> i64 {
    let mut out: string = ""
    let mut i: i64 = 0
    while i < 4 {
        out = string::substring_bytes("abcdefgh", 0, 3)
        i = i + 1
    }
    println(out)
    println("{touch(out)}")
    return 0
}
"#;

#[test]
fn sb15_a_call_after_a_loop_does_not_silence_the_releases_inside_it() {
    let h = Harness::new();
    h.value_row(
        "SB-20",
        SB20_A_SLOT_REASSIGNED_IN_A_LOOP_THAT_ESCAPES_AFTER_IT,
        "abc\n3\n",
        (4, 3),
    );
    h.assert_legs(6);
}

// the share's retain goes when the slot escapes, so a kept release here spends the producer's count
const SB21_A_SHARED_SLOT_REASSIGNED_BEFORE_IT_ESCAPES: &str = r#"
needs std.str

fn build(src: string) -> Vec<string> {
    let mut out = Vec::new()
    let b: string = string::substring_bytes(src, 0, 3)
    let mut a: string = b
    a = string::substring_bytes(src, 3, 6)
    Vec::push(out, a)
    Vec::push(out, b)
    return out
}

fn main() -> i64 {
    let v: Vec<string> = build("abcdefgh")
    let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 3)
    println(poison)
    println(v[0])
    println(v[1])
    return 0
}
"#;

#[test]
fn sb16_a_shared_slot_reassigned_before_it_escapes_answers_what_the_leak_runtime_answers() {
    let h = Harness::new();
    h.differential_row(
        "SB-21",
        SB21_A_SHARED_SLOT_REASSIGNED_BEFORE_IT_ESCAPES,
        "ZZZ\ndef\nabc\n",
        (4, 2),
    );
    h.assert_legs(18);
}

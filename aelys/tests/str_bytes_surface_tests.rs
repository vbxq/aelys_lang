// a title stating nothing allocates still pins a non-zero count, since printing moved from the raw counter

use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::cell::Cell;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
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
        (0, 0),
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
    h.value_row("SB-2", SB2_ITERATION, "10\n1375\n", (0, 0));
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
    h.value_row("SB-3", SB3_NOGC_BYTES, "195\n", (0, 0));
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
    h.value_row("SB-4", SB4_SUBSTRING_BYTES, "élé\nphant\n5\n", (2, 2));
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

#[test]
fn sb5_a_bound_builtin_substring_and_a_bound_library_call_result_are_both_released() {
    let h = Harness::new();
    h.value_row("SB-5-concat", SB5_SUBSTRING_CONCAT, "hello\n", (10, 10));
    h.value_row("SB-5-built", SB5_SUBSTRING_BYTES, "hello\n", (1, 1));
    h.value_row("SB-5-library", SB5_SUBSTRING_LIBRARY, "hello\n", (1, 1));
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
    h.value_row("SB-8-counts", SB8_CHARACTERS_AND_BYTES, "4\n3\n", (0, 0));
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
        (0, 0),
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
        (3, 3),
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
        (4, 4),
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
        (6, 6),
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
        (4, 4),
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
        (4, 4),
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
        (4, 4),
    );
    h.assert_legs(18);
}

const SB22_UNBOUND_TEMPORARIES: &str = r#"
needs std.str

fn id(s: string) -> string {
    return s
}

fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 3 {
        n = n + ("{i}" + "!").len
        if "{i}" == "1" { n = n + 100 }
        Vec::push(v, "k" + "{i}")
        let kept: string = id("r" + "{i}")
        let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 2)
        println(kept + poison)
        println("row {i} of {n}")
        i = i + 1
    }
    let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 2)
    println(v[0] + v[1] + v[2] + poison)
    return 0
}
"#;

#[test]
fn sb17_an_unbound_temporary_is_released_after_its_last_read_and_never_when_handed_on() {
    let h = Harness::new();
    h.differential_row(
        "SB-22",
        SB22_UNBOUND_TEMPORARIES,
        "r0ZZ\nrow 0 of 2\nr1ZZ\nrow 1 of 104\nr2ZZ\nrow 2 of 106\nk0k1k2ZZ\n",
        (47, 47),
    );
    h.assert_legs(18);
}

const SB23_SCALAR_PRINTS: &str = r#"
fn pair(a: string, b: string) -> string {
    return a + "|" + b
}

fn main() -> i64 {
    let x: i64 = 11
    let y: i64 = 22
    let f: f64 = 2.5
    let c: char = 'é'
    let u: u8 = 200
    let mut i: i64 = 0
    while i < 2 {
        println("{x}")
        println(y)
        println(f)
        println(c)
        println(true)
        println(u)
        println(pair("{x}", "{y}"))
        i = i + 1
    }
    return 0
}
"#;

#[test]
fn sb18_a_scalar_println_takes_the_frame_buffer_and_two_interpolated_arguments_stay_apart() {
    let h = Harness::new();
    h.differential_row(
        "SB-23",
        SB23_SCALAR_PRINTS,
        "11\n22\n2.5\né\ntrue\n200\n11|22\n11\n22\n2.5\né\ntrue\n200\n11|22\n",
        (8, 8),
    );
    h.assert_legs(18);
}

const SB24_BREAK_AND_CONTINUE: &str = r#"
needs std.str

fn main() -> i64 {
    let outer: string = "out" + "{7}"
    let mut v: Vec<string> = Vec::new()
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 6 {
        i = i + 1
        let s: string = "s" + "{i}"
        if i % 2 == 0 {
            let kept: string = "k" + "{i}"
            Vec::push(v, kept)
            continue
        }
        let mut j: i64 = 0
        while j < 4 {
            let t: string = "t" + "{j}"
            j = j + 1
            if j == 3 {
                n = n + t.len
                break
            }
            n = n + t.len + s.len
        }
        if i == 5 {
            let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 2)
            println(s + poison)
            break
        }
    }
    let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 2)
    println(outer + poison)
    println(n)
    println(v[0] + v[1])
    return 0
}
"#;

#[test]
fn sb19_break_and_continue_release_the_loop_body_strings_and_nothing_outside_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-24",
        SB24_BREAK_AND_CONTINUE,
        "s5ZZ\nout7ZZ\n30\nk2k4\n",
        (40, 40),
    );
    h.assert_legs(18);
}

const SB25_FRESH_RETURNS: &str = r#"
needs std.str

fn label(i: i64) -> string {
    return "L{i}"
}

fn relabel(i: i64) -> string {
    return label(i)
}

fn id(s: string) -> string {
    return s
}

fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    let base: string = "b" + "{7}"
    let mut i: i64 = 0
    while i < 3 {
        let a: string = label(i)
        let b: string = relabel(i)
        let c: string = id(base)
        let t: string = str.trim(" {i} ")
        Vec::push(v, label(i + 10))
        let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 2)
        println(a + b + c + t + poison)
        println(label(i).len + str.from_int(i).len)
        i = i + 1
    }
    let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 2)
    println(v[0] + v[2] + base + poison)
    return 0
}
"#;

#[test]
fn sb20_a_call_that_returns_fresh_bytes_is_owned_and_a_call_that_returns_its_argument_is_not() {
    let h = Harness::new();
    h.differential_row(
        "SB-25",
        SB25_FRESH_RETURNS,
        "L0L0b70ZZ\n3\nL1L1b71ZZ\n3\nL2L2b72ZZ\n3\nL10L12b7ZZ\n",
        (61, 61),
    );
    h.assert_legs(18);
}

const SB26_THE_ORIGINAL_BLOCKER: &str = r#"
fn main() -> i64 {
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < TURNS {
        let s: string = "row " + "{i}"
        n = n + s.len
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[cfg(target_os = "linux")]
fn peak_rss_kib(exe: &Path, alloc: Option<&str>) -> (i32, i64) {
    #[repr(C)]
    struct RUsage {
        utime: [i64; 2],
        stime: [i64; 2],
        maxrss: i64,
        rest: [i64; 13],
    }
    unsafe extern "C" {
        fn wait4(pid: i32, status: *mut i32, options: i32, usage: *mut RUsage) -> i32;
    }
    let mut cmd = Command::new(exe);
    cmd.stdout(Stdio::null()).stderr(Stdio::null());
    if let Some(a) = alloc {
        cmd.env("AELYS_ALLOC", a);
    }
    let child = cmd.spawn().expect("spawn the measured exe");
    let pid = child.id() as i32;
    let mut status = 0;
    let mut usage = RUsage {
        utime: [0; 2],
        stime: [0; 2],
        maxrss: 0,
        rest: [0; 13],
    };
    let reaped = unsafe { wait4(pid, &mut status, 0, &mut usage) };
    assert_eq!(reaped, pid, "wait4 must reap the child it measures");
    let code = if status & 0x7f == 0 {
        (status >> 8) & 0xff
    } else {
        128 + (status & 0x7f)
    };
    (code, usage.maxrss)
}

#[cfg(target_os = "linux")]
#[test]
fn sb21_the_original_blocker_keeps_a_flat_peak_from_two_hundred_thousand_to_two_million_turns() {
    const VARIANTS: &[(&str, RuntimeVariant)] = &[
        ("leak", RuntimeVariant::Leak),
        ("rc", RuntimeVariant::Rc),
        ("rc+cycles", RuntimeVariant::RcCycles),
    ];
    let h = Harness::new();
    for (tag, opt) in LEVELS {
        for (runtime, variant) in VARIANTS {
            let mut peaks: Vec<(u64, &str, i64)> = Vec::new();
            for turns in [200_000u64, 2_000_000] {
                let src = SB26_THE_ORIGINAL_BLOCKER.replace("TURNS", &turns.to_string());
                let leg = format!("{tag}_{runtime}_{turns}");
                let root = h.stage("SB-26", &leg, &src);
                let Some(exe) = h.compile_at("SB-26", &leg, &root, *opt, *variant) else {
                    assert!(
                        linker_skip_declared(),
                        "SB-26: no linker, and AELYS_ALLOW_LINKER_SKIP is not set; a skipped \
                         peak carries no runtime evidence at all"
                    );
                    return;
                };
                for (alloc_name, alloc) in ALLOCATORS {
                    let (code, peak) = peak_rss_kib(&exe, *alloc);
                    h.legs.set(h.legs.get() + 1);
                    assert_eq!(
                        code, 0,
                        "SB-26 at {tag}/{alloc_name}/{runtime}/{turns} must exit 0"
                    );
                    peaks.push((turns, *alloc_name, peak));
                }
            }
            for (alloc_name, _) in ALLOCATORS {
                let at = |turns: u64| {
                    peaks
                        .iter()
                        .find(|(t, a, _)| *t == turns && a == alloc_name)
                        .map(|(_, _, p)| *p)
                        .expect("both turn counts were measured")
                };
                let growth = at(2_000_000) - at(200_000);
                if *runtime == "leak" {
                    assert!(
                        growth > 65_536,
                        "SB-26 at {tag}/{alloc_name}: the leak runtime keeps every string, so its \
                         peak must grow by more than 64 MiB over 1.8e6 more turns or this row \
                         cannot see a leak at all; it grew by {growth} KiB"
                    );
                } else {
                    assert!(
                        growth < 4_096,
                        "SB-26 at {tag}/{alloc_name}/{runtime}: the bound and the unbound string \
                         of each turn are both released, so the peak must stay within 4 MiB from \
                         2e5 to 2e6 turns; it grew by {growth} KiB"
                    );
                }
            }
        }
    }
    h.assert_legs(LEVELS.len() * 3 * 2 * ALLOCATORS.len());
}

const SB27_ARM_SCOPES: &str = r#"
needs std.str

enum K { A, B }

fn kind(i: i64) -> K {
    if i % 3 == 2 { return K::A }
    return K::B
}

fn label(i: i64) -> string {
    return "L{i}"
}

fn pick(p: string) -> string {
    return p
}

fn main() -> i64 {
    let mut hold: string = "h" + "{0}"
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 6 {
        i = i + 1
        let w: i64 = if i == 4 {
            let t: string = str.trim("  x{i}  ")
            t.len
        } else {
            0
        }
        n = n + w
        match kind(i) {
            K::A => {
                let a: string = "a" + "{i}"
                let b: string = label(i)
                println(a + b)
            }
            K::B => {
                hold = label(i + 10)
                continue
            }
        }
    }
    let z: string = pick("Z" + "{9}")
    println(hold)
    println(z)
    println(n)
    return 0
}
"#;

#[test]
fn sb22_a_string_bound_in_an_arm_is_released_at_the_arm_end_and_never_by_a_later_continue() {
    let h = Harness::new();
    h.differential_row(
        "SB-27",
        SB27_ARM_SCOPES,
        "a2L2\na5L5\nL16\nZ9\n2\n",
        (26, 26),
    );
    h.assert_legs(18);
}

const SB28_REASSIGNED_FROM_A_BORROW: &str = r#"
needs split from std.str

fn pick(p: string) -> string {
    return p
}

fn junk(k: i64) -> i64 {
    let mut t: i64 = 0
    let mut j: i64 = 0
    while j < 8 {
        let z: string = "ZZZZZZZZ" + "{k}"
        t = t + z.len
        j = j + 1
    }
    return t
}

fn main() -> i64 {
    let parts: Vec<string> = split("alpha,beta,gamma", ",")
    let keep: string = "keep-" + "{1}"
    let mut last: string = "none" + "{0}"
    let mut cur: string = "cur-" + "{2}"
    for p in Vec::as_slice(parts) {
        last = p
        cur = pick(keep)
    }
    let k: i64 = junk(3)
    for q in Vec::as_slice(parts) {
        println(q)
    }
    println(last)
    println(cur)
    println(keep)
    println(k)
    return 0
}
"#;

#[test]
fn sb23_a_slot_reassigned_from_a_borrowed_value_in_a_loop_takes_its_own_share() {
    let h = Harness::new();
    h.differential_row(
        "SB-28",
        SB28_REASSIGNED_FROM_A_BORROW,
        "alpha\nbeta\ngamma\ngamma\nkeep-1\nkeep-1\n72\n",
        (26, 26),
    );
    h.assert_legs(18);
}

const SB29_BREAK_IN_A_NESTED_FN: &str = r#"
fn main() -> i64 {
    let mut i: i64 = 0
    while i < 3 {
        fn helper(n: i64) -> i64 {
            let piece: string = "p" + "{n}"
            if n == 1 { break }
            println(piece)
            return n
        }
        println(helper(i))
        i = i + 1
    }
    return 0
}
"#;

const SB29_CONTINUE_IN_A_LAMBDA: &str = r#"
fn main() -> i64 {
    let mut i: i64 = 0
    while i < 3 {
        let f = fn(n: i64) -> i64 {
            let piece: string = "p" + "{n}"
            if n == 1 { continue }
            println(piece)
            return n
        }
        println(f(i))
        i = i + 1
    }
    return 0
}
"#;

#[test]
fn sb24_break_or_continue_inside_a_nested_function_or_a_lambda_is_refused() {
    let h = Harness::new();
    h.rejects(
        "SB-29-fn",
        SB29_BREAK_IN_A_NESTED_FN,
        "E0904",
        "break statement outside of loop",
    );
    h.rejects(
        "SB-29-lambda",
        SB29_CONTINUE_IN_A_LAMBDA,
        "E0904",
        "continue statement outside of loop",
    );
    h.assert_legs(2);
}

const SB30_BLOCK_TAILS: &str = r#"
fn label(i: i64) -> string {
    return "L{i}"
}

fn turns(k: i64) -> i64 {
    return k * 2 + 1
}

fn main() -> i64 {
    let mut hold: string = "h" + "{0}"
    let mut n: i64 = 0
    let mut i: i64 = 0
    let last: i64 = turns(1)
    while i < last {
        let x: string = {
            let t: string = "row " + "{i}"
            t
        }
        n = n + x.len
        n = n + {
            let u: string = "uu" + "{i}"
            u
        }.len
        hold = {
            let w: string = "w" + "{i}"
            n = n + w.len
            label(i)
        }
        println({
            let p: string = "p" + "{i}"
            p
        })
        let y: string = {
            let q: string = "row" + "{i}"
            q
        } + ("Q" + "{i}")
        println(y)
        let poison: string = "ZZZZZZZZ" + "{i}"
        n = n + poison.len
        i = i + 1
    }
    println(hold)
    println(n)
    return 0
}
"#;

#[test]
fn sb25_a_block_hands_its_tail_string_to_the_statement_that_uses_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-30",
        SB30_BLOCK_TAILS,
        "p0\nrow0Q0\np1\nrow1Q1\np2\nrow2Q2\nL2\n57\n",
        (53, 53),
    );
    h.assert_legs(18);
}

const SB31_MANAGED_LOCALS_IN_ARMS: &str = r#"
struct W { r: Rc<i64> }

enum K { A, B }

fn kind(i: i64) -> K {
    if i % 2 == 0 { return K::A }
    return K::B
}

fn main() -> i64 {
    let mut hold: string = "h" + "{0}"
    let tail: string = "yyyyy" + "{1}"
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 4 {
        match kind(i) {
            K::A => {
                let v: Vec<i64> = Vec::new()
                Vec::push(v, 40 + i)
                let r: Rc<i64> = Rc::new(i)
                let c: W = W { r: Rc::new(1000) }
                n = n + v[0] + Rc::get(r) + Rc::get(c.r)
            }
            K::B => {
                hold = "xx" + tail
            }
        }
        let w: i64 = if i % 2 == 1 {
            let u: Vec<i64> = vec[i, i]
            u[0]
        } else {
            7
        }
        n = n + w
        println({
            let b: Vec<i64> = vec[i, 2 * i]
            b[1]
        })
        if i == 1 {
            let q: i64 = 5
            n = n + q
        }
        i = i + 1
    }
    let p: string = "ZZZZZZZZ" + tail
    println(hold)
    println(p)
    println(n)
    return 0
}
"#;

#[test]
fn sb26_a_vec_or_rc_bound_in_an_arm_is_released_at_the_arm_end_and_on_no_other_path() {
    let h = Harness::new();
    h.differential_row(
        "SB-31",
        SB31_MANAGED_LOCALS_IN_ARMS,
        "0\n2\n4\n6\nxxyyyyy1\nZZZZZZZZyyyyy1\n2107\n",
        (19, 19),
    );
    h.assert_legs(18);
}

const SB32_CARRIERS_ACROSS_A_NESTED_FN: &str = r#"
struct W { r: Rc<i64> }

fn main() -> i64 {
    let w: W = W { r: Rc::new(7) }
    fn helper(a: W, c: W, d: W) -> i64 {
        return Rc::get(d.r) + 1
    }
    println(helper(w, w, w))
    let f = fn(a: W, c: W, d: W) -> i64 {
        return Rc::get(d.r) + 2
    }
    println(f(w, w, w))
    let p: Rc<i64> = Rc::new(99)
    println(Rc::get(w.r))
    println(Rc::get(p))
    return 0
}
"#;

#[test]
fn sb27_a_nested_function_or_a_lambda_never_releases_the_carriers_of_its_parent() {
    let h = Harness::new();
    h.differential_row(
        "SB-32",
        SB32_CARRIERS_ACROSS_A_NESTED_FN,
        "8\n9\n7\n99\n",
        (2, 2),
    );
    h.assert_legs(18);
}

const SB33_MANAGED_BLOCK_RESULTS: &str = r#"
struct W { r: Rc<i64> }

fn add(x: Rc<i64>, y: Rc<i64>) -> i64 {
    return Rc::get(x) * 10 + Rc::get(y)
}

fn main() -> i64 {
    println(add({
        let s: W = W { r: Rc::new(1) }
        s.r
    }, {
        let t: W = W { r: Rc::new(2) }
        t.r
    }))
    return 0
}
"#;

#[test]
fn sb28_a_block_whose_result_is_managed_keeps_the_locals_it_may_point_into() {
    let h = Harness::new();
    h.differential_row("SB-33", SB33_MANAGED_BLOCK_RESULTS, "12\n", (2, 2));
    h.assert_legs(18);
}

const SB34_NESTED_FN_IN_A_GENERIC: &str = r#"
fn outer<T>(x: T) -> T {
    fn inner() -> i64 {
        return 1
    }
    let k: i64 = inner()
    let y: T = x
    return y
}

fn main() -> i64 {
    println(outer(41) + 1)
    return 0
}
"#;

#[test]
fn sb29_a_nested_function_leaves_the_type_parameters_of_its_generic_parent_in_scope() {
    let h = Harness::new();
    h.differential_row("SB-34", SB34_NESTED_FN_IN_A_GENERIC, "42\n", (0, 0));
    h.assert_legs(18);
}

const SB35_VIEW_INTO_A_DEEPER_RC: &str = r#"
fn main() -> i64 {
    let s: &[i64] = {
        let a: Rc<[i64; 2]> = Rc::new([1, 2])
        Rc::get(a)[0..2]
    }
    let p: Rc<[i64; 2]> = Rc::new([9, 9])
    println(s[0] * 1000 + Rc::get(p)[0])
    return 0
}
"#;

#[test]
fn sb30_a_block_whose_result_is_a_view_keeps_the_rc_it_points_into() {
    let h = Harness::new();
    h.differential_row("SB-35", SB35_VIEW_INTO_A_DEEPER_RC, "1009\n", (2, 2));
    h.assert_legs(18);
}

const SB36_GENERIC_BLOCK_RESULT: &str = r#"
struct W { r: Rc<i64> }

fn idw(w: W) -> W {
    return w
}

fn getw(w: W) -> i64 {
    return Rc::get(w.r)
}

fn through<T>(k: i64, mk: fn(i64) -> W, f: fn(W) -> T, g: fn(T) -> i64) -> i64 {
    let r: T = {
        let t: W = mk(k)
        f(t)
    }
    let p: Rc<i64> = Rc::new(9)
    return g(r) * 1000 + Rc::get(p)
}

fn mkw(k: i64) -> W {
    return W { r: Rc::new(k) }
}

fn main() -> i64 {
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < 3 {
        n = n + through(i, mkw, idw, getw)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn sb31_a_block_whose_result_is_a_type_parameter_keeps_the_locals_it_may_point_into() {
    let h = Harness::new();
    h.differential_row("SB-36", SB36_GENERIC_BLOCK_RESULT, "3027\n", (6, 6));
    h.assert_legs(18);
}

const SB37_GENERIC_ENUM_BLOCK_RESULT: &str = r#"
fn ident(r: &i64) -> &i64 {
    return r
}

fn read(o: Option<&i64>) -> i64 {
    return match o {
        Option::Some(r) => *r,
        Option::None => 0
    }
}

fn through<T>(k: i64, f: fn(&i64) -> T, g: fn(Option<T>) -> i64) -> i64 {
    let r: Option<T> = {
        let a: Rc<i64> = Rc::new(k)
        Option::Some(f(&Rc::get(a)))
    }
    let p: Rc<i64> = Rc::new(9)
    return g(r) * 1000 + Rc::get(p)
}

fn main() -> i64 {
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < 3 {
        n = n + through(i, ident, read)
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn sb32_a_block_whose_result_wraps_a_type_parameter_keeps_the_locals_it_may_point_into() {
    let h = Harness::new();
    h.differential_row("SB-37", SB37_GENERIC_ENUM_BLOCK_RESULT, "3027\n", (6, 6));
    h.assert_legs(18);
}

const SB38_BINDINGS_THAT_OWN: &str = r#"
needs std.str

fn mk(i: i64) -> string {
    let s: string = "mk" + "{i}"
    return s
}

fn pick(p: string, i: i64) -> string {
    let mut acc: string = p
    acc = acc + "{i}"
    return acc
}

fn main() -> i64 {
    let mut n: i64 = 0
    let mut hold: string = "h" + "{0}"
    let mut i: i64 = 0
    while i < 3 {
        let a: string = "a" + "{i}"
        let b: string = a
        hold = b
        let t: string = mk(i)
        let u: string = pick(t, i)
        let r: string = str.repeat("r", 2)
        for c in a {
            if c == 'a' { n = n + 1 }
        }
        let mut m: string = "m" + "{i}"
        for c in m {
            if c == 'm' { n = n + 1 }
        }
        m = "q" + "{i}"
        for c in "w" + "{i}" {
            if c == 'w' { n = n + 1 }
        }
        let poison: string = "ZZZZZZZZ" + "{i}"
        println(a + b + t + u + r + m)
        n = n + poison.len
        i = i + 1
    }
    let late: string = "YYYYYYYY" + "{n}"
    println(hold)
    println(n + late.len)
    return 0
}
"#;

#[test]
fn sb33_a_copy_a_mutable_binding_a_string_loop_and_a_named_return_each_keep_one_share() {
    let h = Harness::new();
    h.differential_row(
        "SB-38",
        SB38_BINDINGS_THAT_OWN,
        "a0a0mk0mk00rrq0\na1a1mk1mk11rrq1\na2a2mk2mk22rrq2\na2\n46\n",
        (67, 67),
    );
    h.assert_legs(18);
}

const SB39_OWNED_STRING_LOOPS_LEFT_EARLY: &str = r#"
fn first_vowel(s: string) -> i64 {
    let mut n: i64 = 0
    for c in s + "{n}" {
        if c == 'e' { return n }
        n = n + 1
    }
    return -1
}

fn main() -> i64 {
    let mut total: i64 = 0
    let mut i: i64 = 0
    while i < 3 {
        let mut m: string = "abc" + "{i}"
        for c in m {
            if c == 'b' { continue }
            if c == 'c' { break }
            total = total + 1
        }
        m = "zz" + "{i}"
        for c in "xy" + "{i}" {
            if c == 'y' { break }
            total = total + 10
        }
        total = total + first_vowel("hello")
        let poison: string = "ZZZZZZZZ" + "{i}"
        println(m + poison)
        i = i + 1
    }
    println(total)
    return 0
}
"#;

#[test]
fn sb34_an_owned_string_loop_left_by_break_continue_or_return_releases_its_copy_once() {
    let h = Harness::new();
    h.differential_row(
        "SB-39",
        SB39_OWNED_STRING_LOOPS_LEFT_EARLY,
        "zz0ZZZZZZZZ0\nzz1ZZZZZZZZ1\nzz2ZZZZZZZZ2\n36\n",
        (33, 33),
    );
    h.assert_legs(18);
}

const SB41_STRING_ARM_RESULTS: &str = r#"
enum K { A, B }

fn kind(i: i64) -> K {
    if i % 2 == 0 { return K::A }
    return K::B
}

fn pick(i: i64) -> string {
    return if i % 2 == 0 { "even" + "{i}" } else { "odd" + "{i}" }
}

fn width(s: string) -> i64 {
    return s.len
}

fn measured(i: i64) -> i64 {
    return (if i % 2 == 0 { "wide" + "{i}" } else { "w" + "{i}" }).len + width("x")
}

fn main() -> i64 {
    let keep: string = "k" + "{7}"
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 6 {
        i = i + 1
        let s: string = if i % 3 == 0 { "row " + "{i}" } else { keep }
        let t: string = match kind(i) {
            K::A => "a" + "{i}",
            K::B => {
                let u: string = "b" + "{i}"
                u
            }
        }
        if i > 4 { continue }
        let poison: string = "ZZZZZZZZ" + "{i}"
        println(s + t + pick(i) + poison)
        n = n + measured(i) + (if i % 2 == 0 { s } else { t }).len
    }
    println(keep)
    println(n)
    return 0
}
"#;

#[test]
fn sb35_an_if_or_a_match_that_yields_a_string_owns_it_even_on_a_return_path() {
    let h = Harness::new();
    h.differential_row(
        "SB-41",
        SB41_STRING_ARM_RESULTS,
        "k7b1odd1ZZZZZZZZ1\nk7a2even2ZZZZZZZZ2\nrow 3b3odd3ZZZZZZZZ3\nk7a4even4ZZZZZZZZ4\nk7\n26\n",
        (54, 54),
    );
    h.assert_legs(18);
}

const SB42_POSED_STRINGS_ACROSS_BRANCHES: &str = r#"
fn tail_of(p: string, i: i64) -> string {
    return {
        let k: i64 = i
        p
    }
}

fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 6 {
        let x: string = "x" + "{i}"
        let y: string = "yy" + "{i}"
        n = n + {
            let k: i64 = i
            x
        }.len + (if i > 2 { 1 } else { 0 })
        println((if i % 2 == 0 { x } else { y }) + (if i % 3 == 0 { y } else { x }))
        if (if i % 2 == 0 { x } else { y }).len > 2 {
            n = n + 100
        }
        let w: i64 = if i > 3 {
            ({
                let k: i64 = i
                y
            }).len
        } else {
            0
        }
        let t: string = tail_of("lit", i)
        let poison: string = "ZZZZZZZZ" + "{i}"
        n = n + w + t.len + poison.len
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn sb36_a_string_posed_before_a_branch_in_the_same_statement_is_released_once() {
    let h = Harness::new();
    h.differential_row(
        "SB-42",
        SB42_POSED_STRINGS_ACROSS_BRANCHES,
        "x0yy0\nyy1x1\nx2x2\nyy3yy3\nx4x4\nyy5x5\n393\n",
        (42, 42),
    );
    h.assert_legs(18);
}

const SB43_STRING_TEMPS_IN_A_WHILE_CONDITION: &str = r#"
enum K { A, B }

fn kind(i: i64) -> K {
    if i % 2 == 0 { return K::A }
    return K::B
}

fn main() -> i64 {
    let mut i: i64 = 0
    let mut x: string = "xx" + "{0}"
    let mut y: string = "y" + "{0}"
    let keep: string = "keep" + "{0}"
    let mut n: i64 = 0
    while i < 10 && match kind(i) {
        K::A => (if i % 4 == 0 { x } else { y }).len > 1,
        K::B => ({
            let k: i64 = i
            if k > 2 { x } else { keep }
        }).len > 1 && (if i > 7 { "zz" + "{i}" } else { y }).len > 0
    } {
        let before: string = x
        x = "xx" + "{i + 1}"
        y = "y" + "{i + 1}"
        let poison: string = "PPPPPPPPPPPPPPPPPPPPPPPPPP" + "{i}"
        let poison2: string = "QQQQQQQQQQQQQQQQQQQQQQQQQQ" + "{i}"
        n = n + poison.len + poison2.len
        println(before + " " + x + " " + y)
        if (if i % 3 == 0 { x } else { before }).len > 3 && ({
            let k: i64 = i
            y
        }).len > 1 {
            n = n + 1
        }
        i = i + 1
    }
    println(keep)
    println(n)
    return 0
}
"#;

#[test]
fn sb37_a_string_made_in_a_while_condition_is_released_before_the_branch() {
    let h = Harness::new();
    h.differential_row(
        "SB-43",
        SB43_STRING_TEMPS_IN_A_WHILE_CONDITION,
        "xx0 xx1 y1\nxx1 xx2 y2\nxx2 xx3 y3\nxx3 xx4 y4\nxx4 xx5 y5\nxx5 xx6 y6\nxx6 xx7 y7\nxx7 xx8 y8\nxx8 xx9 y9\nxx9 xx10 y10\nkeep0\n541\n",
        (128, 128),
    );
    h.assert_legs(18);
}

const SB44_STRING_TEMPS_ACROSS_A_SHORT_CIRCUIT: &str = r#"
enum K { A, B }

fn kind(i: i64) -> K {
    if i % 2 == 0 { return K::A }
    return K::B
}

fn main() -> i64 {
    let mut i: i64 = 0
    let mut hits: i64 = 0
    let keep: string = "keep" + "{0}"
    while i < 8 {
        i = i + 1
        let x: string = "xx" + "{i}"
        let y: string = "y" + "{i}"
        let a: bool = (if i % 2 == 0 { x } else { y }).len == 3 && ({
            let k: i64 = i
            x
        }).len > 2
        let b: bool = i > 3 || (if i % 3 == 0 { "ab" + "{i}" } else { y }).len == 2 && match kind(i) {
            K::A => (if i > 1 { x } else { keep }).len > 0,
            K::B => ({
                let q: string = "q" + "{i}"
                q
            }).len == 2
        }
        let s: string = (if a { x } else { y }) + (if b && (if i > 5 { keep } else { x }).len > 3 { "!" } else { "?" })
        let poison: string = "PPPPPPPPPPPPPPPPPPPPPPPPPP" + "{i}"
        let poison2: string = "QQQQQQQQQQQQQQQQQQQQQQQQQQ" + "{i}"
        if a { hits = hits + 1 }
        if b { hits = hits + 10 }
        println(s + " " + "{poison.len + poison2.len}")
    }
    println(keep)
    println(hits)
    return 0
}
"#;

#[test]
fn sb38_a_string_made_on_one_side_of_a_short_circuit_is_released_on_that_side() {
    let h = Harness::new();
    h.differential_row(
        "SB-44",
        SB44_STRING_TEMPS_ACROSS_A_SHORT_CIRCUIT,
        "y1? 54\nxx2? 54\ny3? 54\nxx4? 54\ny5? 54\nxx6! 54\ny7! 54\nxx8! 54\nkeep0\n74\n",
        (102, 102),
    );
    h.assert_legs(18);
}

const SB45_STRING_TEMPS_ON_THE_ERROR_PATH_OF_A_TRY: &str = r#"
enum Result<T, E> { Ok(T), Err(E) }

fn probe(i: i64) -> Result<i64, i64> {
    if i % 2 == 0 { return Result::Err(i) }
    return Result::Ok(i)
}

fn stepf(i: i64) -> Result<i64, i64> {
    let x: string = "xx" + "{i}"
    let n: i64 = (if i % 3 == 0 { x } else { "f" + "{i}" }).len + probe(i)?
    return Result::Ok(n)
}

fn main() -> i64 {
    let mut i: i64 = 0
    let mut total: i64 = 0
    while i < 12 {
        let r: i64 = match stepf(i) {
            Result::Ok(v) => v,
            Result::Err(e) => 0 - 1
        }
        total = total + r
        i = i + 1
    }
    println(total)
    return 0
}
"#;

#[test]
fn sb39_a_string_posed_before_a_failing_question_mark_is_released_on_the_error_path() {
    let h = Harness::new();
    h.differential_row(
        "SB-45",
        SB45_STRING_TEMPS_ON_THE_ERROR_PATH_OF_A_TRY,
        "45\n",
        (40, 40),
    );
    h.assert_legs(18);
}

const SB46_STRING_TEMPS_AT_NON_LOCAL_EXITS: &str = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum K { A, B, C }

fn kind(i: i64) -> K {
    if i % 3 == 0 { return K::A }
    if i % 3 == 1 { return K::B }
    return K::C
}

fn probe(i: i64) -> Result<i64, i64> {
    if i % 4 == 3 { return Result::Err(i) }
    return Result::Ok(i)
}

fn ret_in_inner_loop(i: i64, p: string) -> i64 {
    let x: string = "rx" + "{i}"
    let n: i64 = (if i % 2 == 0 { x } else { "f" + "{i}" }).len + {
        let mut k: i64 = 0
        while k < 5 {
            let q: i64 = (if k % 2 == 0 { p } else { x }).len + match kind(k) {
                K::A => {
                    if k == i { return k * 1000 + (if k > 2 { x } else { p }).len }
                    1
                },
                K::B => {
                    if k > i { break }
                    2
                },
                K::C => {
                    if k == 2 { k = k + 1
                        continue }
                    3
                }
            }
            k = k + q - q + 1
        }
        k
    }
    return n
}

fn try_in_cond(i: i64, p: string) -> Result<i64, i64> {
    let mut k: i64 = 0
    let mut acc: i64 = 0
    while k < 6 && (if k % 2 == 0 { p } else { "c" + "{k}" }).len > 0 && probe(i + k)? >= 0 {
        acc = acc + (match kind(k) {
            K::A => if probe(k)? > 1 { p } else { "a" + "{k}" },
            K::B => p,
            K::C => ({
                let z: i64 = probe(k + 1)?
                "cc" + "{z}"
            })
        }).len
        k = k + 1
    }
    return Result::Ok(acc)
}

fn cont_in_bounds(i: i64, p: string) -> i64 {
    let mut outer: i64 = 0
    let mut acc: i64 = 0
    while outer < 4 {
        outer = outer + 1
        let t: i64 = (if outer % 2 == 0 { p } else { "o" + "{outer}" }).len + {
            let mut s: i64 = 0
            for j in 0..({
                if outer == 3 { continue }
                (if j0(outer) { p } else { "bb" + "{outer}" }).len
            }) {
                s = s + j
            }
            s
        }
        while { if outer == 4 { break }; (if outer > 1 { p } else { "w" }).len } > 100 {
            acc = acc + 1
        }
        acc = acc + t
    }
    return acc
}

fn j0(n: i64) -> bool {
    return n % 2 == 0
}

fn main() -> i64 {
    let mut i: i64 = 0
    let mut total: i64 = 0
    while i < 6 {
        let p: string = "pp" + "{i}"
        let a: i64 = ret_in_inner_loop(i, "lit")
        let b: i64 = match try_in_cond(i, "lit2") {
            Result::Ok(v) => v,
            Result::Err(e) => 0 - e
        }
        let c: i64 = cont_in_bounds(i, "lit3")
        let poison: string = "PPPPPPPPPPPPPPPPPPPPPPPPPP" + "{i}"
        let poison2: string = "QQQQQQQQQQQQQQQQQQQQQQQQQQ" + "{i}"
        total = total + a + b + c + poison.len + poison2.len + p.len
        println("{a} {b} {c}")
        i = i + 1
    }
    println(total)
    return 0
}
"#;

#[test]
fn sb40_a_return_break_or_continue_releases_exactly_the_strings_posed_on_its_path() {
    let h = Harness::new();
    h.differential_row(
        "SB-46",
        SB46_STRING_TEMPS_AT_NON_LOCAL_EXITS,
        "3 -3 15\n6 -3 15\n7 -3 15\n3003 -3 15\n8 -3 15\n7 -7 15\n3444\n",
        (152, 152),
    );
    h.assert_legs(18);
}

const SB47_STRING_TEMPS_ACROSS_STATEMENT_BRANCHES: &str = r#"
fn main() -> i64 {
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < 6 {
        let x: string = "xx" + "{i}"
        let y: string = "y" + "{i}"
        n = n + (if i % 2 == 0 { x } else { y }).len + {
            if i > 2 { n = n + 1 }
            0
        }
        i = i + 1
    }
    println(n)
    return 0
}
"#;

#[test]
fn sb41_a_string_posed_before_a_statement_if_or_while_in_a_block_is_released_once() {
    let h = Harness::new();
    h.differential_row(
        "SB-47",
        SB47_STRING_TEMPS_ACROSS_STATEMENT_BRANCHES,
        "15\n",
        (24, 24),
    );
    h.assert_legs(18);
}

const SB48_A_RETURNED_STRING_THAT_WAS_ALSO_STORED: &str = r#"
fn add(v: &mut Vec<string>, x: string) -> string {
    let s: string = x + "!"
    Vec::push(*v, s)
    return s
}
fn main() -> i64 {
    let mut names: Vec<string> = Vec::new()
    let mut i: i64 = 0
    while i < 3 {
        println(add(&mut names, "n" + "{i}"))
        i = i + 1
    }
    let p: string = "QQ" + "{8}"
    println(names[0])
    println(names[2])
    println(p)
    return 0
}
"#;

#[test]
fn sb42_a_string_both_stored_and_returned_hands_the_caller_a_share_of_its_own() {
    let h = Harness::new();
    h.differential_row(
        "SB-48",
        SB48_A_RETURNED_STRING_THAT_WAS_ALSO_STORED,
        "n0!\nn1!\nn2!\nn0!\nn2!\nQQ8\n",
        (12, 12),
    );
    h.assert_legs(18);
}

const SB49_AN_ARM_TAIL_THAT_WAS_ALSO_STORED: &str = r#"
let mut G: string = "init"
fn f(s: string, c: bool) -> i64 {
    let y: string = if c {
        let x: string = s + "!"
        G = x
        x
    } else {
        "n"
    }
    println(y)
    return y.len
}
fn main() -> i64 {
    let n: i64 = f("abc", true)
    let p: string = "QQ" + "{8}"
    println(G)
    println(p)
    println(n)
    return 0
}
"#;

#[test]
fn sb43_an_arm_tail_both_stored_and_moved_out_takes_a_share_of_its_own() {
    let h = Harness::new();
    h.differential_row(
        "SB-49",
        SB49_AN_ARM_TAIL_THAT_WAS_ALSO_STORED,
        "abc!\nabc!\nQQ8\n4\n",
        (3, 2),
    );
    h.assert_legs(18);
}

const SB50_A_LOOP_OVER_AN_OWNED_BLOCK_TAIL: &str = r#"
fn main() -> i64 {
    let s: string = "ab" + "{1}"
    let mut n: i64 = 0
    for c in {
        let x: string = s + "!"
        x
    } {
        let p: string = "QQ" + "{8}"
        print(c)
        n = n + p.len
    }
    println("")
    println("{n}")
    return 0
}
"#;

#[test]
fn sb44_a_loop_over_a_block_tail_takes_the_tail_instead_of_borrowing_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-50",
        SB50_A_LOOP_OVER_AN_OWNED_BLOCK_TAIL,
        "ab1!\n12\n",
        (11, 11),
    );
    h.assert_legs(18);
}

const SB51_A_SLOT_THAT_ESCAPES_BY_REFERENCE_BEFORE_IT_IS_REASSIGNED: &str = r#"
needs std.str

fn peek(s: &string) -> i64 {
    return (*s).len
}

fn build(src: string) -> i64 {
    let mut a: string = string::substring_bytes(src, 0, 3)
    let n: i64 = peek(&a)
    a = string::substring_bytes(src, 3, 6)
    println(a)
    return n
}

fn main() -> i64 {
    let n: i64 = build("abcdefgh")
    let poison: string = string::substring_bytes("ZZZZZZZZ", 0, 3)
    println(poison)
    println("{n}")
    return 0
}
"#;

#[test]
fn sb45_a_slot_whose_address_escapes_before_it_is_reassigned_answers_what_leak_answers() {
    let h = Harness::new();
    h.differential_row(
        "SB-51",
        SB51_A_SLOT_THAT_ESCAPES_BY_REFERENCE_BEFORE_IT_IS_REASSIGNED,
        "def\nZZZ\n3\n",
        (3, 3),
    );
    h.assert_legs(18);
}

const SB52_A_LOOP_SLOT_THAT_ESCAPES_BY_REFERENCE_AFTER_THE_LOOP: &str = r#"
needs std.str

fn touch(s: &string) -> i64 {
    return (*s).len
}

fn main() -> i64 {
    let mut out: string = ""
    let mut i: i64 = 0
    while i < 4 {
        out = string::substring_bytes("abcdefgh", 0, 3)
        i = i + 1
    }
    println(out)
    println("{touch(&out)}")
    return 0
}
"#;

#[test]
fn sb46_a_slot_that_escapes_by_reference_after_a_loop_keeps_the_releases_inside_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-52",
        SB52_A_LOOP_SLOT_THAT_ESCAPES_BY_REFERENCE_AFTER_THE_LOOP,
        "abc\n3\n",
        (4, 4),
    );
    h.assert_legs(18);
}

const SB53_A_PARAMETER_STORED_IN_A_STRUCT_AN_ENUM_OR_AN_ARRAY: &str = r#"
struct P { name: string }
fn mk(s: string) -> P { return P { name: s } }
fn wrap(s: string) -> Option<string> { return Option::Some(s) }
fn arr(s: string) -> [string; 2] { return [s, "c"] }
fn fill(s: string) -> [string; 3] { return [s; 3] }
fn main() -> i64 {
    let p: P = mk("ab" + "{1}")
    let o: Option<string> = wrap("ab" + "{2}")
    let a: [string; 2] = arr("ab" + "{3}")
    let b: [string; 3] = fill("ab" + "{4}")
    let q: string = "QQ" + "{8}"
    let r: string = "QQ" + "{9}"
    println(p.name)
    match o {
        Option::Some(x) => println(x),
        Option::None => println("none"),
    }
    println(a[0])
    println(b[0])
    println(b[2])
    println(q)
    println(r)
    return 0
}
"#;

#[test]
fn sb47_a_parameter_stored_in_a_struct_an_enum_or_an_array_takes_a_share_of_its_own() {
    let h = Harness::new();
    h.differential_row(
        "SB-53",
        SB53_A_PARAMETER_STORED_IN_A_STRUCT_AN_ENUM_OR_AN_ARRAY,
        "ab1\nab2\nab3\nab4\nab4\nQQ8\nQQ9\n",
        (12, 12),
    );
    h.assert_legs(18);
}

const SB54_A_PARAMETER_STORED_THROUGH_A_REFERENCE_OR_INTO_A_GLOBAL: &str = r#"
struct P { name: string }
let mut G: string = "init"
fn put(v: &mut Vec<string>, s: string) { Vec::push(*v, s) }
fn setn(p: &mut P, s: string) { (*p).name = s }
fn seti(a: &mut [string], s: string) { a[0] = s }
fn setd(r: &mut string, s: string) { *r = s }
fn setg(s: string) { G = s }
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    let mut p: P = P { name: "x" }
    let mut a: [string; 2] = ["x", "y"]
    let mut d: string = "x"
    put(&mut v, "ab" + "{1}")
    setn(&mut p, "ab" + "{2}")
    seti(a[..], "ab" + "{3}")
    setd(&mut d, "ab" + "{4}")
    setg("ab" + "{5}")
    let q: string = "QQ" + "{8}"
    let r: string = "QQ" + "{9}"
    println(v[0])
    println(p.name)
    println(a[0])
    println(d)
    println(G)
    println(q)
    println(r)
    return 0
}
"#;

#[test]
fn sb48_a_parameter_stored_through_a_reference_or_into_a_global_takes_a_share_of_its_own() {
    let h = Harness::new();
    h.differential_row(
        "SB-54",
        SB54_A_PARAMETER_STORED_THROUGH_A_REFERENCE_OR_INTO_A_GLOBAL,
        "ab1\nab2\nab3\nab4\nab5\nQQ8\nQQ9\n",
        (15, 13),
    );
    h.assert_legs(18);
}

const SB55_A_STRING_KEPT_BY_AN_RC_OR_A_CLOSURE: &str = r#"
fn mkf(s: string) -> fn() -> string { return fn() -> string { return s } }
fn mkg() -> fn(string) -> string {
    let mut held: string = "x"
    return fn(u: string) -> string {
        let old: string = held
        held = u
        return old
    }
}
fn main() -> i64 {
    let mut t: string = "ab" + "{1}"
    let b: Rc<string> = Rc::new(t)
    t = "zz"
    let f = mkf("ab" + "{2}")
    let g = mkg()
    let first: string = g("ab" + "{3}")
    let q: string = "QQ" + "{8}"
    let r: string = "QQ" + "{9}"
    println(Rc::get(b))
    println(f())
    println(first)
    println(g("zz"))
    println(q)
    println(r)
    return 0
}
"#;

#[test]
fn sb49_a_string_kept_by_an_rc_or_a_closure_takes_a_share_of_its_own() {
    let h = Harness::new();
    h.differential_row(
        "SB-55",
        SB55_A_STRING_KEPT_BY_AN_RC_OR_A_CLOSURE,
        "ab1\nab2\nx\nab3\nQQ8\nQQ9\n",
        (13, 8),
    );
    h.assert_legs(18);
}

const SB56_A_PARAMETER_HANDED_TO_A_GENERIC_CALLEE: &str = r#"
fn keep<T>(x: T) -> Option<T> { return Option::Some(x) }
fn through(s: string) -> Option<string> { return keep(s) }
fn main() -> i64 {
    let a: Option<string> = through("ab" + "{1}")
    let s: string = "ab" + "{2}"
    let b: Option<string> = through(s)
    let q: string = "QQ" + "{8}"
    let r: string = "QQ" + "{9}"
    match a {
        Option::Some(x) => println(x),
        Option::None => println("none"),
    }
    match b {
        Option::Some(x) => println(x),
        Option::None => println("none"),
    }
    println(s)
    println(q)
    println(r)
    return 0
}
"#;

#[test]
fn sb50_a_parameter_handed_to_a_generic_callee_takes_a_share_of_its_own() {
    let h = Harness::new();
    h.differential_row(
        "SB-56",
        SB56_A_PARAMETER_HANDED_TO_A_GENERIC_CALLEE,
        "ab1\nab2\nab2\nQQ8\nQQ9\n",
        (8, 8),
    );
    h.assert_legs(18);
}

const SB57_ARGUMENTS_TO_AELYS_FUNCTIONS_AND_FUNCTION_VALUES: &str = r#"
struct P { name: string }
fn consume(s: string) -> i64 { return s.len }
fn mk(s: string) -> P { return P { name: s } }
fn main() -> i64 {
    let mut total: i64 = 0
    let mut i: i64 = 0
    while i < 4 {
        total = total + consume("x" + "{i}")
        let s: string = "y" + "{i}"
        total = total + consume(s)
        let f = consume
        total = total + f(s)
        i = i + 1
    }
    let m = mk
    let p: P = m("ab" + "{1}")
    let q: string = "QQ" + "{8}"
    println("{total}")
    println(p.name)
    println(q)
    return 0
}
"#;

#[test]
fn sb51_an_argument_to_an_aelys_function_or_a_function_value_is_released_by_its_caller() {
    let h = Harness::new();
    h.differential_row(
        "SB-57",
        SB57_ARGUMENTS_TO_AELYS_FUNCTIONS_AND_FUNCTION_VALUES,
        "24\nab1\nQQ8\n",
        (20, 20),
    );
    h.assert_legs(18);
}

const SB58_A_PARAMETER_STORED_IN_A_VEC_LITERAL_OR_A_DECLARED_ARRAY: &str = r#"
fn mkv(s: string) -> Vec<string> { return vec[s, "y"] }
fn mka(s: string) -> [string; 2] {
    let a: [string; 2] = [s, "y"]
    return a
}
fn main() -> i64 {
    let v: Vec<string> = mkv("ab" + "{1}")
    let a: [string; 2] = mka("ab" + "{2}")
    let q: string = "QQ" + "{8}"
    let r: string = "QQ" + "{9}"
    println(v[0])
    println(a[0])
    println(q)
    println(r)
    return 0
}
"#;

#[test]
fn sb52_a_parameter_stored_in_a_vec_literal_or_a_declared_array_takes_a_share_of_its_own() {
    let h = Harness::new();
    h.differential_row(
        "SB-58",
        SB58_A_PARAMETER_STORED_IN_A_VEC_LITERAL_OR_A_DECLARED_ARRAY,
        "ab1\nab2\nQQ8\nQQ9\n",
        (9, 9),
    );
    h.assert_legs(18);
}

const SB59_A_PARAMETER_HANDED_TO_AN_IMPORTED_GENERIC: &str = r#"
needs std.result

fn keep(s: string) -> Result<string, i64> { return result.ok(s) }

fn main() -> i64 {
    let a: Result<string, i64> = keep("ab" + "{1}")
    let b: Result<string, i64> = {
        let t: string = "ab" + "{2}"
        keep(t)
    }
    let q: string = "QQ" + "{8}"
    let r: string = "QQ" + "{9}"
    match a {
        Result::Ok(x) => println(x),
        Result::Err(_) => println("e"),
    }
    match b {
        Result::Ok(x) => println(x),
        Result::Err(_) => println("e"),
    }
    println(q)
    println(r)
    return 0
}
"#;

#[test]
fn sb53_a_parameter_handed_to_an_imported_generic_takes_a_share_of_its_own() {
    let h = Harness::new();
    h.differential_row(
        "SB-59",
        SB59_A_PARAMETER_HANDED_TO_AN_IMPORTED_GENERIC,
        "ab1\nab2\nQQ8\nQQ9\n",
        (8, 8),
    );
    h.assert_legs(18);
}

const SB60_A_VEC_OF_STRINGS_ACROSS_COPIES_STORES_AND_POPS: &str = r#"
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, "ab" + "{1}")
    Vec::push(v, "ab" + "{2}")
    Vec::push(v, "ab" + "{3}")
    let w: Vec<string> = v
    v[0] = "ab" + "{9}"
    v[1] = v[1]
    v[2] = v[1]
    let a: string = Vec::pop(v)
    println(Vec::pop(v))
    let q: string = "QQ" + "{8}"
    let r: string = "QQ" + "{7}"
    println(a)
    println(v[0])
    println(w[0])
    println(w[1])
    println(w[2])
    println(q)
    println(r)
    return 0
}
"#;

#[test]
fn sb54_a_vec_of_strings_owns_its_elements_across_copies_stores_and_pops() {
    let h = Harness::new();
    h.differential_row(
        "SB-60",
        SB60_A_VEC_OF_STRINGS_ACROSS_COPIES_STORES_AND_POPS,
        "ab2\nab2\nab9\nab1\nab2\nab3\nQQ8\nQQ7\n",
        (14, 14),
    );
    h.assert_legs(18);
}

const SB61_A_VEC_OF_STRINGS_THROUGH_PARAMETERS_AND_REASSIGNMENT: &str = r#"
fn edit(mut v: Vec<string>) -> string {
    v[0] = "ab" + "{7}"
    return v[0]
}
fn poke(v: &mut Vec<string>) {
    (*v)[1] = "ab" + "{8}"
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, "ab" + "{1}")
    Vec::push(v, "ab" + "{2}")
    let w: Vec<string> = v
    let e: string = edit(v)
    poke(&mut v)
    let keep: string = v[0]
    v = Vec::new()
    Vec::push(v, "ab" + "{3}")
    let q: string = "QQ" + "{5}"
    let r: string = "QQ" + "{6}"
    println(e)
    println(keep)
    println(v[0])
    println(w[0])
    println(w[1])
    println(q)
    println(r)
    return 0
}
"#;

#[test]
fn sb55_a_vec_of_strings_keeps_its_elements_through_parameters_and_reassignment() {
    let h = Harness::new();
    h.differential_row(
        "SB-61",
        SB61_A_VEC_OF_STRINGS_THROUGH_PARAMETERS_AND_REASSIGNMENT,
        "ab7\nab1\nab3\nab1\nab2\nQQ5\nQQ6\n",
        (18, 18),
    );
    h.assert_legs(18);
}

const SB62_UNBOUND_VEC_RESULTS: &str = r#"
needs std.str

fn mk(n: i64) -> Vec<string> {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, "k" + "{n}")
    Vec::push(v, "m" + "{n}")
    return v
}
fn count(v: Vec<string>) -> i64 { return Vec::len(v) }
fn main() -> i64 {
    let mut total: i64 = 0
    let mut i: i64 = 0
    while i < 3 {
        let line: string = "a{i},bb{i},c{i}"
        total = total + str.char_count(str.split(line, ",")[1])
        let parts: Vec<string> = str.split(line, ",")
        total = total + parts[2].len
        mk(i)
        total = total + count(mk(i))
        total = total + Vec::len(mk(i))
        i = i + 1
    }
    let q: string = "QQ" + "{8}"
    println("{total}")
    println(q)
    return 0
}
"#;

#[test]
fn sb56_an_unbound_vec_result_is_released_at_the_end_of_its_statement() {
    let h = Harness::new();
    h.differential_row("SB-62", SB62_UNBOUND_VEC_RESULTS, "27\nQQ8\n", (95, 95));
    h.assert_legs(18);
}

const SB63_VECS_OF_A_LOOP_BODY_AT_BREAK_AND_CONTINUE: &str = r#"
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 6 {
        i = i + 1
        let mut v: Vec<string> = Vec::new()
        Vec::push(v, "ab" + "{i}")
        if i % 2 == 0 {
            continue
        }
        if i == 5 {
            break
        }
        n = n + v[0].len
    }
    let q: string = "QQ" + "{8}"
    println("{n}")
    println(q)
    return 0
}
"#;

#[test]
fn sb57_break_and_continue_release_the_vecs_of_the_loop_body() {
    let h = Harness::new();
    h.differential_row(
        "SB-63",
        SB63_VECS_OF_A_LOOP_BODY_AT_BREAK_AND_CONTINUE,
        "6\nQQ8\n",
        (17, 17),
    );
    h.assert_legs(18);
}

const SB64_A_VEC_KEPT_OPEN_PAST_ITS_ARM: &str = r#"
fn first<T>(x: T, y: T) -> i64 {
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < 4 {
        i = i + 1
        let odd: bool = i % 2 == 1
        let r: T = if odd {
            let a: Vec<i64> = vec[i, 2]
            n = n + a[0]
            x
        } else {
            y
        }
        if !odd {
            continue
        }
        n = n + 100
    }
    return n
}
fn main() -> i64 {
    let b: Vec<i64> = vec[9, 9]
    let mut i: i64 = 0
    let mut n: i64 = 0
    while i < 4 {
        i = i + 1
        let odd: bool = i % 2 == 1
        let sl: &[i64] = if odd {
            let a: Vec<i64> = vec[i, 2]
            n = n + a[0]
            b[..]
        } else {
            b[..]
        }
        if !odd {
            continue
        }
        n = n + sl[0]
    }
    println(first(1, 2))
    println(n)
    return 0
}
"#;

#[test]
fn sb58_a_vec_kept_open_past_its_arm_is_never_released_by_a_later_continue() {
    let h = Harness::new();
    h.differential_row(
        "SB-64",
        SB64_A_VEC_KEPT_OPEN_PAST_ITS_ARM,
        "204\n22\n",
        (5, 5),
    );
    h.assert_legs(18);
}

const SB65_A_VEC_ELEMENT_READ_BEFORE_A_SIBLING_THAT_FREES_IT: &str = r#"
fn clobber(v: &mut Vec<string>) -> i64 {
    (*v)[0] = "zz" + "{7}"
    let p: string = "QQ" + "{8}"
    return 1
}
fn show(x: string, k: i64) -> i64 {
    println(x)
    return k
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, "a" + "{1}")
    let r: string = v[0] + {
        v[0] = "zz" + "{2}"
        let p: string = "QQ" + "{3}"
        p
    }
    println(r)
    show(v[0], clobber(&mut v))
    println("{v[0]}|{clobber(&mut v)}")
    return 0
}
"#;

#[test]
fn sb59_a_vec_element_read_before_a_sibling_that_frees_it_is_pinned() {
    let h = Harness::new();
    h.differential_row(
        "SB-65",
        SB65_A_VEC_ELEMENT_READ_BEFORE_A_SIBLING_THAT_FREES_IT,
        "a1QQ3\nzz2\nzz7|1\n",
        (19, 19),
    );
    h.assert_legs(18);
}

const SB66_A_GENERIC_THAT_PUSHES_A_STRING: &str = r#"
fn put<T>(v: &mut Vec<T>, x: T) { Vec::push(*v, x) }
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    put(&mut v, "ab" + "{1}")
    println(v[0])
    return 0
}
"#;

#[test]
fn sb60_a_generic_that_pushes_a_string_takes_its_share() {
    let h = Harness::new();
    h.differential_row(
        "SB-66",
        SB66_A_GENERIC_THAT_PUSHES_A_STRING,
        "ab1\n",
        (3, 3),
    );
    h.assert_legs(18);
}

const SB67_A_GENERIC_THAT_WRAPS_A_BORROWED_ELEMENT: &str = r#"
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

fn get<T>(v: &Vec<T>, i: i64) -> Option<T> {
    if i < 0 || i >= Vec::len(*v) { return Option::None }
    return Option::Some((*v)[i])
}
fn lookup(names: &Vec<string>, i: i64) -> string {
    match get(names, i) {
        Option::Some(x) => x,
        Option::None => "?",
    }
}
fn load() -> Option<string> {
    let names: Vec<string> = vec[victim(1), victim(2), victim(3)]
    return get(&names, 1)
}
fn main() -> i64 {
    let mut names: Vec<string> = vec[victim(4), victim(5)]
    println(lookup(&names, 0))
    let o: Option<string> = load()
    let k = churn()
    match o {
        Option::Some(x) => println(x),
        Option::None => println("none"),
    }
    return 0
}
"#;

#[test]
fn sb61_a_generic_that_wraps_an_element_it_borrows_retains_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-67",
        SB67_A_GENERIC_THAT_WRAPS_A_BORROWED_ELEMENT,
        "ab4\nab2\n",
        (93, 93),
    );
    h.assert_legs(18);
}

const SB68_AN_ELEMENT_READ_BESIDE_A_MUTABLE_REFERENCE_BY_NAME: &str = r#"
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

fn show(s: string, r: &mut Vec<string>) -> i64 {
    (*r)[0] = victim(9)
    let k = churn()
    println(s)
    return 1
}
fn process(r: &mut Vec<string>) -> i64 {
    return show((*r)[0], r)
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(1))
    process(&mut v)
    println(v[0])
    return 0
}
"#;

#[test]
fn sb62_an_element_read_is_pinned_when_the_consumer_takes_a_mutable_reference_by_name() {
    let h = Harness::new();
    h.differential_row(
        "SB-68",
        SB68_AN_ELEMENT_READ_BESIDE_A_MUTABLE_REFERENCE_BY_NAME,
        "ab1\nab9\n",
        (86, 86),
    );
    h.assert_legs(18);
}

const SB69_COMPOUND_STORES_THAT_FREE_THEIR_ELEMENT: &str = r#"
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

fn clobs(r: &mut Vec<string>) -> string {
    (*r)[0] = victim(9)
    let k = churn()
    return "z"
}
fn mk(n: i64) -> Vec<string> {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(n))
    return v
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(1))
    v[0] = v[0] + clobs(&mut v)
    println(v[0])
    v[0] = v[0] + {
        v = mk(5)
        let k = churn()
        "y"
    }
    println(v[0])
    return 0
}
"#;

#[test]
fn sb63_a_compound_store_pins_the_element_it_reads_before_its_right_side_runs() {
    let h = Harness::new();
    h.differential_row(
        "SB-69",
        SB69_COMPOUND_STORES_THAT_FREE_THEIR_ELEMENT,
        "ab1z\nab1zy\n",
        (172, 172),
    );
    h.assert_legs(18);
}

const SB70_BYTE_VIEWS_INDEXED_BY_A_SIBLING_THAT_FREES: &str = r#"
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

fn clob(v: &mut Vec<string>) -> i64 {
    (*v)[0] = victim(9)
    let k = churn()
    return 1
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, "x" + "yz{1}")
    let b: u8 = v[0].bytes[clob(&mut v)]
    let mut s: string = "x" + "wz{1}"
    let c: u8 = s.bytes[{
        s = victim(2)
        let k = churn()
        1
    }]
    println("{b}|{c}")
    return 0
}
"#;

#[test]
fn sb64_a_byte_view_indexed_by_a_sibling_that_frees_its_string_reads_a_pinned_copy() {
    let h = Harness::new();
    h.differential_row(
        "SB-70",
        SB70_BYTE_VIEWS_INDEXED_BY_A_SIBLING_THAT_FREES,
        "121|119\n",
        (177, 177),
    );
    h.assert_legs(18);
}

const SB71_A_GENERIC_THAT_STORES_ONLY_CONCRETE_STRINGS: &str = r#"
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

fn tag<T>(x: T) -> i64 {
    let mut names: Vec<string> = Vec::new()
    Vec::push(names, "a" + "{1}")
    Vec::push(names, "b")
    return Vec::len(names)
}
fn main() -> i64 {
    println("{tag(5)}")
    println("{tag(true)}")
    return 0
}
"#;

#[test]
fn sb65_a_generic_that_stores_only_concrete_strings_is_accepted() {
    let h = Harness::new();
    h.differential_row(
        "SB-71",
        SB71_A_GENERIC_THAT_STORES_ONLY_CONCRETE_STRINGS,
        "2\n2\n",
        (6, 6),
    );
    h.assert_legs(18);
}

const SB72_AN_ARGUMENT_TO_A_GENERIC_READ_BEFORE_A_SIBLING_REASSIGNS_IT: &str = r#"
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

fn keep2<T>(a: T, n: i64) -> Option<T> { return Option::Some(a) }
fn main() -> i64 {
    let mut s: string = victim(1)
    let o: Option<string> = keep2(s, { s = victim(2); 1 })
    let k = churn()
    match o {
        Option::Some(x) => println(x),
        Option::None => println("none"),
    }
    println(s)
    println(k[0])
    return 0
}
"#;

#[test]
fn sb66_an_argument_to_a_generic_callee_takes_its_share_when_it_is_read() {
    let h = Harness::new();
    h.differential_row(
        "SB-72",
        SB72_AN_ARGUMENT_TO_A_GENERIC_READ_BEFORE_A_SIBLING_REASSIGNS_IT,
        "ab1\nab2\nQ10\n",
        (85, 85),
    );
    h.assert_legs(18);
}

const SB73_A_PUSH_ON_A_SHARED_BUFFER: &str = r#"
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

fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    let mut copies: i64 = 0
    let mut i: i64 = 0
    let mut w: Vec<string> = Vec::new()
    while i < 20 {
        Vec::push(v, victim(i))
        if i % 3 == 0 {
            w = v
        }
        i = i + 1
    }
    let k = churn()
    println(v[0] + v[19] + w[0] + w[18])
    println("{Vec::len(w)}")
    println(k[0])
    return 0
}
"#;

#[test]
fn sb67_a_push_on_a_shared_buffer_retains_the_elements_it_copies() {
    let h = Harness::new();
    h.differential_row(
        "SB-73",
        SB73_A_PUSH_ON_A_SHARED_BUFFER,
        "ab0ab19ab0ab18\n19\nQ10\n",
        (133, 133),
    );
    h.assert_legs(18);
}

const SB74_A_POP_ON_A_SHARED_BUFFER: &str = r#"
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

fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(1))
    Vec::push(v, victim(2))
    Vec::push(v, victim(3))
    let w: Vec<string> = v
    let a: string = Vec::pop(v)
    Vec::pop(v)
    let x: Vec<string> = v
    let b: string = Vec::pop(v)
    let k = churn()
    println(a + b)
    println(w[0] + w[1] + w[2])
    println(x[0])
    println("{Vec::len(v)}")
    println(k[0])
    return 0
}
"#;

#[test]
fn sb68_a_pop_on_a_shared_buffer_retains_the_elements_it_copies() {
    let h = Harness::new();
    h.differential_row(
        "SB-74",
        SB74_A_POP_ON_A_SHARED_BUFFER,
        "ab3ab1\nab1ab2ab3\nab1\n0\nQ10\n",
        (93, 93),
    );
    h.assert_legs(18);
}

const SB75_A_GENERIC_THAT_READS_AN_ELEMENT_THEN_REPLACES_THE_VEC: &str = r#"
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

fn take_first<T>(v: &mut Vec<T>, w: Vec<T>) -> T {
    let x: T = (*v)[0]
    *v = w
    return x
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(1))
    let mut w: Vec<string> = Vec::new()
    Vec::push(w, victim(2))
    let s: string = take_first(&mut v, w)
    let k = churn()
    println(s)
    println(v[0])
    return 0
}
"#;

#[test]
fn sb69_a_generic_that_reads_an_element_then_replaces_the_vec_owns_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-75",
        SB75_A_GENERIC_THAT_READS_AN_ELEMENT_THEN_REPLACES_THE_VEC,
        "ab1\nab2\n",
        (87, 87),
    );
    h.assert_legs(18);
}

const SB76_A_GROUPED_BYTES_BASE_BESIDE_AN_INDEX_THAT_FREES_IT: &str = r#"
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

fn clobi(r: &mut Vec<string>) -> i64 {
    (*r)[0] = victim(9)
    let k = churn()
    return 1
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, "x" + "yz{1}")
    let r: &mut Vec<string> = &mut v
    let b: u8 = ((*r)[0].bytes)[clobi(r)]
    let mut s: string = "x" + "yz{1}"
    let c: u8 = (s.bytes)[{ s = victim(2); let k = churn(); 1 }]
    println("{b} {c}")
    return 0
}
"#;

#[test]
fn sb70_a_grouped_bytes_base_is_pinned_while_its_index_frees_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-76",
        SB76_A_GROUPED_BYTES_BASE_BESIDE_AN_INDEX_THAT_FREES_IT,
        "121 121\n",
        (177, 177),
    );
    h.assert_legs(18);
}

const SB77_A_GENERIC_INSTANTIATED_WITH_A_STRUCT_OR_AN_OPTION: &str = r#"
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

struct P { name: string }
fn put<T>(v: &mut Vec<T>, x: T) { Vec::push(*v, x) }
fn main() -> i64 {
    let mut v: Vec<P> = Vec::new()
    put(&mut v, P { name: victim(1) })
    let mut w: Vec<Option<string>> = Vec::new()
    put(&mut w, Option::Some(victim(2)))
    let k = churn()
    println(v[0].name)
    match w[0] {
        Option::Some(x) => println(x),
        Option::None => println("none"),
    }
    return 0
}
"#;

#[test]
fn sb71_a_generic_instantiated_with_a_type_that_only_holds_a_string_is_accepted() {
    let h = Harness::new();
    h.differential_row(
        "SB-77",
        SB77_A_GENERIC_INSTANTIATED_WITH_A_STRUCT_OR_AN_OPTION,
        "ab1\nab2\n",
        (87, 87),
    );
    h.assert_legs(18);
}

const SB78_A_GENERIC_THAT_WRAPS_A_STRING_WITHOUT_A_VEC: &str = r#"
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

fn wrap_local<T>(x: T) -> Option<T> {
    let y: T = x
    return Option::Some(y)
}
fn wrap_pick<T>(c: bool, a: T, b: T) -> Option<T> {
    return Option::Some(pick(c, a, b))
}
fn pick<T>(c: bool, a: T, b: T) -> T { if c { return a } return b }
fn main() -> i64 {
    let o: Option<string> = wrap_local(victim(1))
    let p: Option<string> = wrap_pick(true, victim(2), victim(3))
    let k = churn()
    match o {
        Option::Some(x) => println(x),
        Option::None => println("none"),
    }
    match p {
        Option::Some(x) => println(x),
        Option::None => println("none"),
    }
    return 0
}
"#;

#[test]
fn sb72_a_generic_that_wraps_a_string_without_a_vec_is_accepted() {
    let h = Harness::new();
    h.differential_row(
        "SB-78",
        SB78_A_GENERIC_THAT_WRAPS_A_STRING_WITHOUT_A_VEC,
        "ab1\nab2\n",
        (87, 87),
    );
    h.assert_legs(18);
}

const SB79_A_GENERIC_THAT_WRITES_ONE_STRING_INTO_A_SLICE_OF_A_VEC: &str = r#"
fn fill<T>(s: &mut [T], x: T) {
    for i in 0..s.len { s[i] = x }
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, "a" + "{1}")
    Vec::push(v, "b" + "{2}")
    let m = Vec::try_as_unique_mut_slice(v)
    let x: string = "c" + "{3}"
    fill(m, x)
    println(v[0])
    println(v[1])
    return 0
}
"#;

#[test]
fn sb73_a_generic_that_writes_through_a_slice_of_strings_retains_each_store() {
    let h = Harness::new();
    h.differential_row(
        "SB-79",
        SB79_A_GENERIC_THAT_WRITES_ONE_STRING_INTO_A_SLICE_OF_A_VEC,
        "c3\nc3\n",
        (7, 7),
    );
    h.assert_legs(18);
}

const SB80_A_GENERIC_THAT_WRAPS_AN_ELEMENT_OF_A_SLICE: &str = r#"
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

fn first2<T>(s: &[T]) -> Option<T> {
    if s.len == 0 { return Option::None }
    return Option::Some(s[0])
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(1))
    let o: Option<string> = first2(Vec::as_slice(v))
    v[0] = victim(2)
    let k = churn()
    match o {
        Option::Some(x) => println(x),
        Option::None => println("none"),
    }
    return 0
}
"#;

#[test]
fn sb74_a_generic_that_carries_a_slice_element_out_in_an_enum_retains_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-80",
        SB80_A_GENERIC_THAT_WRAPS_AN_ELEMENT_OF_A_SLICE,
        "ab1\n",
        (86, 86),
    );
    h.assert_legs(18);
}

const SB81_A_GENERIC_THAT_ONLY_READS_A_SLICE_OF_A_VEC: &str = r#"
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

fn idx<T: eq>(s: &[T], x: T) -> i64 {
    for i in 0..s.len {
        if s[i] == x { return i }
    }
    return -1
}
fn first<T>(s: &[T]) -> T { return s[0] }
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(1))
    Vec::push(v, victim(2))
    let i: i64 = idx(Vec::as_slice(v), "ab" + "{2}")
    let o: string = first(Vec::as_slice(v))
    v[0] = victim(3)
    let k = churn()
    println("{i} " + o + " " + v[0])
    return 0
}
"#;

#[test]
fn sb75_a_generic_that_only_reads_a_slice_of_strings_is_accepted() {
    let h = Harness::new();
    h.differential_row(
        "SB-81",
        SB81_A_GENERIC_THAT_ONLY_READS_A_SLICE_OF_A_VEC,
        "1 ab1 ab3\n",
        (95, 95),
    );
    h.assert_legs(18);
}

const SB82_AN_OPERAND_READ_BEFORE_A_SIBLING_REWRITES_ITS_LOCAL: &str = r#"
struct P { a: i64, b: i64 }
fn two(a: i64, b: i64) -> i64 { return a * 10 + b }
fn keep2c(a: string, n: i64) -> string { return a }
fn keep2<T>(a: T, n: i64) -> T { return a }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut x: i64 = 1
    let c: i64 = two(x, { x = 2; 3 })
    x = 1
    let d: i64 = x + { x = 2; 0 }
    x = 1
    let p: P = P { a: x, b: { x = 2; 3 } }
    x = 1
    let v: Vec<i64> = vec[x, { x = 2; 3 }]
    x = 1
    let a: [i64; 2] = [x, { x = 2; 3 }]
    x = 1
    let f: string = "{x}{ { x = 2; 3 } }"
    let mut s: string = victim(1)
    let t: string = keep2c(s, { s = victim(2); 1 })
    let u: string = keep2(s, { s = victim(3); 1 })
    println("{c} {d} {p.a}{p.b} {v[0]}{v[1]} {a[0]}{a[1]} {f}")
    println(t + u + s)
    return 0
}
"#;

#[test]
fn sb76_an_operand_keeps_the_value_it_read_when_a_later_sibling_rewrites_its_local() {
    let h = Harness::new();
    h.differential_row(
        "SB-82",
        SB82_AN_OPERAND_READ_BEFORE_A_SIBLING_REWRITES_ITS_LOCAL,
        "13 1 13 13 13 13\nab1ab2ab3\n",
        (33, 33),
    );
    h.assert_legs(18);
}

const SB83_A_GENERIC_IDENTITY_IN_A_LOOP: &str = r#"
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

fn keep<T>(x: T) -> T { return x }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 1000 {
        let s: string = keep(victim(i))
        n = n + s.len
        i = i + 1
    }
    let k = churn()
    println("{n}")
    return 0
}
"#;

#[test]
fn sb77_a_string_through_a_generic_identity_is_released_every_turn() {
    let h = Harness::new();
    h.differential_row(
        "SB-83",
        SB83_A_GENERIC_IDENTITY_IN_A_LOOP,
        "4890\n",
        (2081, 2081),
    );
    h.assert_legs(18);
}

const SB84_GENERICS_INSTANTIATED_WITH_SCALARS: &str = r#"
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

fn keep<T>(x: T) -> T { return x }
fn pick<T>(c: bool, a: T, b: T) -> T { if c { return a } return b }
fn put<T>(v: &mut Vec<T>, x: T) { Vec::push(*v, x) }
fn main() -> i64 {
    let mut v: Vec<f64> = Vec::new()
    put(&mut v, keep(1.5))
    let b: bool = pick(true, keep(true), false)
    let n: i64 = pick(false, 1, keep(41))
    println("{v[0]} {b} {n}")
    return 0
}
"#;

#[test]
fn sb78_a_generic_instantiated_with_a_scalar_counts_nothing() {
    let h = Harness::new();
    h.differential_row(
        "SB-84",
        SB84_GENERICS_INSTANTIATED_WITH_SCALARS,
        "1.5 true 41\n",
        (7, 7),
    );
    h.assert_legs(18);
}

const SB85_A_GENERIC_ELEMENT_READ_BESIDE_A_CALL_THAT_REPLACES_IT: &str = r#"
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

fn g<T>(v: &mut Vec<T>, x: T) -> T {
    (*v)[0] = x
    let k = churn()
    return (*v)[0]
}
fn same<T: eq>(v: &mut Vec<T>, x: T) -> bool {
    return (*v)[0] == g(v, x)
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(1))
    println("{same(&mut v, victim(1))}")
    return 0
}
"#;

#[test]
fn sb79_a_generic_element_read_is_pinned_while_a_sibling_replaces_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-85",
        SB85_A_GENERIC_ELEMENT_READ_BESIDE_A_CALL_THAT_REPLACES_IT,
        "true\n",
        (86, 86),
    );
    h.assert_legs(18);
}

const SB86_A_GENERIC_MUT_PARAMETER_REASSIGNED_FROM_AN_ELEMENT: &str = r#"
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

fn grab<T>(mut x: T, v: &Vec<T>) -> Option<T> {
    x = (*v)[0]
    return Option::Some(x)
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(1))
    let o: Option<string> = grab(victim(5), &v)
    v[0] = victim(2)
    let k = churn()
    match o {
        Option::Some(x) => println(x),
        Option::None => println("none"),
    }
    return 0
}
"#;

#[test]
fn sb80_a_generic_mut_parameter_owns_what_it_is_reassigned() {
    let h = Harness::new();
    h.differential_row(
        "SB-86",
        SB86_A_GENERIC_MUT_PARAMETER_REASSIGNED_FROM_AN_ELEMENT,
        "ab1\n",
        (88, 88),
    );
    h.assert_legs(18);
}

const SB87_A_GENERIC_POP: &str = r#"
fn take<T>(v: &mut Vec<T>) -> T { return Vec::pop(*v) }
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, "a" + "{1}")
    Vec::push(v, "b" + "{2}")
    let w: Vec<string> = v
    let x: string = take(&mut v)
    println(x)
    println(w[0])
    println(v[0])
    return 0
}
"#;

#[test]
fn sb81_a_generic_pop_hands_its_element_over() {
    let h = Harness::new();
    h.differential_row("SB-87", SB87_A_GENERIC_POP, "b2\na1\na1\n", (6, 6));
    h.assert_legs(18);
}

const SB88_OPERANDS_REWRITTEN_THROUGH_A_REFERENCE_OR_AS_AN_AGGREGATE: &str = r#"
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

struct P { a: i64, b: i64 }
fn two(a: i64, b: i64) -> i64 { return a * 10 + b }
fn bump(r: &mut i64) -> i64 { *r = 2; return 3 }
fn bumps(r: &mut string) -> i64 { *r = victim(2); let k = churn(); return 1 }
fn keep2c(a: string, n: i64) -> string { return a }
fn twoa(a: [i64; 2], n: i64) -> i64 { return a[0] * 10 + n }
fn twop(p: P, n: i64) -> i64 { return p.a * 10 + n }
fn twov(v: Vec<i64>, n: i64) -> i64 { return Vec::len(v) * 10 + n }
fn main() -> i64 {
    let mut x: i64 = 1
    let c1: i64 = two(x, bump(&mut x))
    let mut s: string = victim(1)
    let c2: string = keep2c(s, bumps(&mut s))
    let mut a: [i64; 2] = [1, 0]
    let c3: i64 = twoa(a, { a[0] = 2; 3 })
    let mut p: P = P { a: 1, b: 0 }
    let c4: i64 = twop(p, { p.a = 2; 3 })
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 1)
    let c5: i64 = twov(v, { Vec::push(v, 2); 3 })
    let k = churn()
    println("{c1} " + c2 + " {c3} {c4} {c5}")
    return 0
}
"#;

#[test]
fn sb82_an_operand_keeps_its_value_when_a_sibling_lends_it_mutably_or_rewrites_its_aggregate() {
    let h = Harness::new();
    h.differential_row(
        "SB-88",
        SB88_OPERANDS_REWRITTEN_THROUGH_A_REFERENCE_OR_AS_AN_AGGREGATE,
        "13 ab1 13 13 13\n",
        (180, 180),
    );
    h.assert_legs(18);
}

const SB89_UNBOUND_TEMPORARIES_OF_A_TYPE_PARAMETER: &str = r#"
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

fn keep<T>(x: T) -> T { return x }
fn cnt<T>(x: T) -> i64 { return 1 }
fn eqk<T: eq>(a: T, b: T) -> bool { return keep(a) == keep(b) }
fn pass<T>(a: T) -> i64 { return cnt(keep(a)) + cnt(keep(keep(a))) }
fn disc<T>(a: T) -> i64 { keep(a); keep(keep(a)); let n: i64 = cnt(keep(a)); return n }
fn wrap<T>(a: T) -> Option<T> { return Option::Some(keep(a)) }
fn arr<T>(a: T) -> [T; 2] { return [keep(a), keep(a)] }
fn pv<T>(v: &mut Vec<T>, a: T) { Vec::push(*v, keep(a)); (*v)[0] = keep(a) }
fn main() -> i64 {
    let s: string = victim(1)
    let mut v: Vec<string> = Vec::new()
    pv(&mut v, s)
    let mut w: Vec<i64> = Vec::new()
    pv(&mut w, 3)
    let o: Option<string> = wrap(s)
    let a: [string; 2] = arr(s)
    let b: [i64; 2] = arr(4)
    let k = churn()
    println("{eqk(1, 1)} {eqk(s, victim(1))} {pass(1)} {pass(s)} {disc(2.5)} {disc(s)} {w[0]} {b[1]} " + v[0] + a[1])
    match o { Option::Some(x) => println(x), Option::None => println("none") }
    match wrap(7) { Option::Some(x) => println("{x}"), Option::None => println("none") }
    return 0
}
"#;

#[test]
fn sb83_an_unbound_temporary_of_a_type_parameter_is_released_by_its_own_count() {
    let h = Harness::new();
    h.differential_row(
        "SB-89",
        SB89_UNBOUND_TEMPORARIES_OF_A_TYPE_PARAMETER,
        "true true 2 2 1 1 3 4 ab1ab1\nab1\n7\n",
        (110, 110),
    );
    h.assert_legs(18);
}

const SB90_A_MUT_PARAMETER_READ_BEFORE_ITS_REASSIGNMENT: &str = r#"
fn holdc(mut x: i64, y: i64) -> i64 { let z: i64 = x; x = y; return z }
fn holdg<T>(mut x: T, y: T) -> T { let z: T = x; x = y; return z }
fn holdl<T>(a: T, y: T) -> T { let mut x: T = a; let z: T = x; x = y; return z }
fn holdlc(a: i64, y: i64) -> i64 { let mut x: i64 = a; let z: i64 = x; x = y; return z }
fn main() -> i64 {
    println("{holdc(5, 6)} {holdg(5, 6)} {holdl(5, 6)} {holdlc(5, 6)}")
    let mut x: i64 = 5
    let z: i64 = x
    x = 6
    println("{z}")
    return 0
}
"#;

#[test]
fn sb84_a_mut_parameter_read_before_it_is_reassigned_keeps_its_entry_value() {
    let h = Harness::new();
    h.differential_row(
        "SB-90",
        SB90_A_MUT_PARAMETER_READ_BEFORE_ITS_REASSIGNMENT,
        "5 5 5 5\n5\n",
        (10, 10),
    );
    h.assert_legs(18);
}

const SB91_A_MUT_PARAMETER_REWRITTEN_BY_A_SIBLING_OPERAND: &str = r#"
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

fn two(a: i64, b: i64) -> i64 { return a * 10 + b }
fn keep2<T>(a: T, n: i64) -> T { return a }
fn f(mut x: i64, y: i64) -> i64 { return two(x, { x = y; 3 }) }
fn fg<T>(mut x: T, y: T) -> T { return keep2(x, { x = y; 1 }) }
fn fl(mut x: i64, y: i64) -> i64 { let p: i64 = x + { x = y; 0 }; return p }
fn main() -> i64 {
    println("{f(1, 2)} {fg(1, 2)} {fl(1, 2)}")
    let s: string = fg(victim(1), victim(2))
    let k = churn()
    println(s)
    return 0
}
"#;

#[test]
fn sb85_a_mut_parameter_rewritten_by_a_sibling_operand_keeps_the_value_it_read() {
    let h = Harness::new();
    h.differential_row(
        "SB-91",
        SB91_A_MUT_PARAMETER_REWRITTEN_BY_A_SIBLING_OPERAND,
        "13 1 1\nab1\n",
        (92, 92),
    );
    h.assert_legs(18);
}

const SB92_A_READ_THROUGH_A_RETURNED_REFERENCE_BESIDE_A_CALL_THAT_FREES_IT: &str = r#"
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

fn atc(v: &Vec<string>, i: i64) -> &string { return &(*v)[i] }
fn at<T>(v: &Vec<T>, i: i64) -> &T { return &(*v)[i] }
fn clob(v: &mut Vec<string>) -> i64 { (*v)[0] = victim(8); let k = churn(); return 1 }
fn show2(s: string, n: i64) -> string { return s + "{n}" }
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(1))
    let a: string = show2(*at(&v, 0), clob(&mut v))
    let b: string = show2(*atc(&v, 0), clob(&mut v))
    let k = churn()
    println(a + b)
    return 0
}
"#;

#[test]
fn sb86_a_read_through_a_returned_reference_is_pinned_while_a_sibling_frees_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-92",
        SB92_A_READ_THROUGH_A_RETURNED_REFERENCE_BESIDE_A_CALL_THAT_FREES_IT,
        "ab11ab81\n",
        (255, 255),
    );
    h.assert_legs(18);
}

const SB93_A_CALLEE_REWRITTEN_BY_ITS_OWN_ARGUMENT: &str = r#"
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

fn f1(a: i64, b: i64) -> i64 { return 100 + b }
fn f2(a: i64, b: i64) -> i64 { return 200 + b }
fn main() -> i64 {
    let mut f: fn(i64, i64) -> i64 = f1
    println("{f(1, { f = f2; 3 })}")
    return 0
}
"#;

#[test]
fn sb87_a_callee_is_read_before_its_arguments_rewrite_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-93",
        SB93_A_CALLEE_REWRITTEN_BY_ITS_OWN_ARGUMENT,
        "103\n",
        (0, 0),
    );
    h.assert_legs(18);
}

const SB94_A_VEC_READ_THROUGH_A_REFERENCE_BESIDE_A_SIBLING_THAT_REPLACES_IT: &str = r#"
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

fn lenv(v: Vec<string>, n: i64) -> string { let k = churn(); return "{Vec::len(v)}" + v[0] + "{n}" }
fn repl(vr: &mut Vec<string>) -> string { return lenv(*vr, { *vr = Vec::new(); Vec::push(*vr, victim(3)); 1 }) }
fn grow(vr: &mut Vec<string>) -> string {
    return lenv(*vr, { let mut i: i64 = 0; while i < 100 { Vec::push(*vr, victim(i)); i = i + 1 }; 2 })
}
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(1))
    let a: string = repl(&mut v)
    let mut w: Vec<string> = Vec::new()
    Vec::push(w, victim(2))
    let b: string = grow(&mut w)
    let k = churn()
    println(a + " " + b + " {Vec::len(v)} {Vec::len(w)}")
    return 0
}
"#;

#[test]
fn sb88_a_vec_read_through_a_reference_holds_its_buffer_while_a_sibling_replaces_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-94",
        SB94_A_VEC_READ_THROUGH_A_REFERENCE_BESIDE_A_SIBLING_THAT_REPLACES_IT,
        "1ab11 1ab22 1 101\n",
        (469, 469),
    );
    h.assert_legs(18);
}

const SB95_AN_ARRAY_OF_A_TYPE_PARAMETER_REWRITTEN_BY_A_SIBLING: &str = r#"
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

fn firstc(a: [i64; 2], n: i64) -> i64 { return a[0] }
fn gac(mut a: [i64; 2], x: i64) -> i64 { return firstc(a, { a[0] = x; 2 }) }
fn firsts(a: [string; 2], n: i64) -> string { return a[0] }
fn gas(mut a: [string; 2], x: string) -> string { return firsts(a, { a[0] = x; 2 }) }
fn first<T>(a: [T; 2], n: i64) -> T { return a[0] }
fn ga<T>(mut a: [T; 2], x: T) -> T { return first(a, { a[0] = x; 2 }) }
fn gl<T>(x: T, y: T) -> T { let mut a: [T; 2] = [x, x]; return first(a, { a[0] = y; 2 }) }
fn main() -> i64 {
    let k = churn()
    println("{gac([6, 7], 8)} " + gas([victim(3), victim(4)], victim(5)) + " {ga([6, 7], 8)} " + ga([victim(3), victim(4)], victim(5)) + " {gl(1, 2)} " + gl(victim(1), victim(2)))
    return 0
}
"#;

#[test]
fn sb89_an_array_of_a_type_parameter_keeps_the_value_it_read() {
    let h = Harness::new();
    h.differential_row(
        "SB-95",
        SB95_AN_ARRAY_OF_A_TYPE_PARAMETER_REWRITTEN_BY_A_SIBLING,
        "6 ab3 6 ab3 1 ab1\n",
        (110, 110),
    );
    h.assert_legs(18);
}

const SB96_A_VEC_PARAMETER_WITHOUT_AN_ELEMENT_TYPE: &str = r#"
fn setret(mut v: Vec, x: string) -> string { v[0] = x; return v[0] }
fn main() -> i64 {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, "a" + "{1}")
    println(setret(v, "b" + "{2}"))
    return 0
}
"#;

#[test]
fn sb90_a_vec_parameter_without_an_element_type_is_refused_by_the_surface_check() {
    let h = Harness::new();
    h.rejects(
        "SB-96",
        SB96_A_VEC_PARAMETER_WITHOUT_AN_ELEMENT_TYPE,
        "E0412",
        "parameter `v` of `setret` has type `vec_opaque`",
    );
    h.assert_legs(1);
}

const SB97_A_STRING_LENT_BY_ADDRESS_IN_A_LOOP: &str = r#"
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

fn lens(r: &string) -> i64 { return (*r).len }
fn glen<T>(r: &T, n: i64) -> i64 { return n }
fn bump(r: &mut string) { *r = *r + "!" }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 1000 {
        let s: string = victim(i)
        n = n + lens(&s) + glen(&s, 1)
        let mut t: string = victim(i)
        bump(&mut t)
        n = n + t.len
        i = i + 1
    }
    let k = churn()
    println("{n}")
    return 0
}
"#;

#[test]
fn sb91_a_string_whose_address_is_lent_to_an_aelys_callee_is_still_released() {
    let h = Harness::new();
    h.differential_row(
        "SB-97",
        SB97_A_STRING_LENT_BY_ADDRESS_IN_A_LOOP,
        "11780\n",
        (5081, 5081),
    );
    h.assert_legs(18);
}

const SB98_STRUCT_AND_ENUM_TEMPORARIES_IN_A_LONG_LOOP: &str = r#"
struct P { name: string, age: i64 }
fn half(n: i64) -> Option<i64> { if n % 2 == 0 { return Option::Some(n / 2) } return Option::None }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 300000 {
        let p: P = P { name: "p", age: i }
        let q: P = p
        n = n + q.age % 7
        match half(i) {
            Option::Some(h) => { n = n + h % 3 }
            Option::None => { n = n + 1 }
        }
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn sb92_struct_and_enum_temporaries_in_a_loop_do_not_grow_the_frame() {
    let h = Harness::new();
    h.differential_row(
        "SB-98",
        SB98_STRUCT_AND_ENUM_TEMPORARIES_IN_A_LONG_LOOP,
        "1199997\n",
        (0, 0),
    );
    h.assert_legs(18);
}

// a lent address is safe only while a reference cannot be stored, returned, captured or lent out

const SB99_A_REFERENCE_STORED_IN_A_CONTAINER: &str = r#"
struct H { r: &string }
fn main() -> i64 {
    let s: string = "a" + "b"
    let h: H = H { r: &s }
    println(*h.r)
    return 0
}
"#;

const SB99_A_REFERENCE_RETURNED_FROM_A_FUNCTION: &str = r#"
fn dangle() -> &string {
    let s: string = "a" + "b"
    return &s
}
fn main() -> i64 {
    println(*dangle())
    return 0
}
"#;

const SB99_A_CLOSURE_THAT_CAPTURES_A_REFERENCE: &str = r#"
fn main() -> i64 {
    let s: string = "a" + "b"
    let r: &string = &s
    let f = fn(n: i64) -> i64 { return n + (*r).len }
    println("{f(1)}")
    return 0
}
"#;

const SB99_AN_EXTERNAL_DECLARATION_AS_A_VALUE: &str = r#"
unsafe extern fn puts(s: &string) -> i64
fn main() -> i64 {
    let f: fn(&string) -> i64 = puts
    let s: string = "a" + "b"
    println("{f(&s)}")
    return 0
}
"#;

const SB99_A_REFERENCE_HANDED_TO_AN_EXTERNAL_CALL: &str = r#"
unsafe extern fn takes(s: &string) -> i64
fn main() -> i64 {
    let s: string = "a" + "b"
    println("{takes(&s)}")
    return 0
}
"#;

#[test]
fn sb93_a_reference_cannot_be_stored_returned_captured_or_handed_to_an_extern() {
    let h = Harness::new();
    h.rejects(
        "SB-99A",
        SB99_A_REFERENCE_STORED_IN_A_CONTAINER,
        "E0714",
        "references stored into aggregate containers",
    );
    h.rejects(
        "SB-99B",
        SB99_A_REFERENCE_RETURNED_FROM_A_FUNCTION,
        "E0721",
        "function returns a reference to local `s`",
    );
    h.rejects(
        "SB-99C",
        SB99_A_CLOSURE_THAT_CAPTURES_A_REFERENCE,
        "E0725",
        "a closure that captures a reference",
    );
    h.rejects(
        "SB-99D",
        SB99_AN_EXTERNAL_DECLARATION_AS_A_VALUE,
        "E0616",
        "may only be called directly, never used as a value",
    );
    h.rejects(
        "SB-99E",
        SB99_A_REFERENCE_HANDED_TO_AN_EXTERNAL_CALL,
        "E0617",
        "must appear inside an `unsafe { }` block",
    );
    h.assert_legs(5);
}

const SB101_A_STRUCT_COPIED_IN_A_LOOP: &str = r#"
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

struct P { name: string, city: string, age: i64 }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 1000 {
        let p: P = P { name: victim(i), city: victim(i + 1), age: i }
        let q: P = p
        n = n + q.name.len + p.city.len
        i = i + 1
    }
    let k = churn()
    println("{n}")
    return 0
}
"#;

#[test]
fn sb94_a_struct_that_holds_strings_releases_them_on_every_copy() {
    let h = Harness::new();
    h.differential_row(
        "SB-101",
        SB101_A_STRUCT_COPIED_IN_A_LOOP,
        "9783\n",
        (4081, 4081),
    );
    h.assert_legs(18);
}

const SB102_A_FIELD_STORE_IN_A_LOOP: &str = r#"
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

struct P { name: string, age: i64 }
fn main() -> i64 {
    let mut p: P = P { name: victim(0), age: 0 }
    let mut i: i64 = 1
    while i < 1000 {
        p.name = victim(i)
        i = i + 1
    }
    let k = churn()
    println(p.name)
    return 0
}
"#;

#[test]
fn sb95_a_field_store_releases_the_string_it_replaces() {
    let h = Harness::new();
    h.differential_row(
        "SB-102",
        SB102_A_FIELD_STORE_IN_A_LOOP,
        "ab999\n",
        (2081, 2081),
    );
    h.assert_legs(18);
}

const SB103_AN_ARRAY_ELEMENT_STORE_IN_A_LOOP: &str = r#"
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

fn main() -> i64 {
    let mut a: [string; 3] = [victim(0), victim(1), victim(2)]
    let mut i: i64 = 3
    while i < 1000 {
        a[i % 3] = victim(i)
        i = i + 1
    }
    let k = churn()
    println(a[0] + a[1] + a[2])
    return 0
}
"#;

#[test]
fn sb96_an_array_of_strings_releases_what_it_holds_and_what_it_replaces() {
    let h = Harness::new();
    h.differential_row(
        "SB-103",
        SB103_AN_ARRAY_ELEMENT_STORE_IN_A_LOOP,
        "ab999ab997ab998\n",
        (2083, 2083),
    );
    h.assert_legs(18);
}

const SB104_A_FOR_OVER_ARRAYS_OF_CARRIERS: &str = r#"
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

struct P { name: string, age: i64 }
fn main() -> i64 {
    let arr: [P; 2] = [P { name: victim(1), age: 1 }, P { name: victim(2), age: 2 }]
    let mut n: i64 = 0
    for p in arr {
        n = n + p.name.len + p.age
    }
    let mut brr: [string; 3] = [victim(3), victim(4), victim(5)]
    let mut acc: string = ""
    for s in brr {
        brr[1] = victim(9)
        let k = churn()
        acc = acc + s
    }
    println("{n} " + acc)
    return 0
}
"#;

#[test]
fn sb97_a_for_over_an_array_holds_the_collection_and_borrows_its_element() {
    let h = Harness::new();
    h.differential_row(
        "SB-104",
        SB104_A_FOR_OVER_ARRAYS_OF_CARRIERS,
        "9 ab3ab4ab5\n",
        (265, 265),
    );
    h.assert_legs(18);
}

const SB105_A_VEC_OF_STRUCTS_ACROSS_COPIES_STORES_AND_POPS: &str = r#"
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
struct P { name: string, age: i64 }
struct Q { r: Rc<i64>, name: string }

fn setp<T>(v: &mut Vec<T>, i: i64, x: T) { (*v)[i] = x }
fn takep<T>(v: &mut Vec<T>) -> T { return Vec::pop(*v) }

fn main() -> i64 {
    let mut v: Vec<P> = Vec::new()
    Vec::push(v, P { name: victim(1), age: 1 })
    let w: Vec<P> = v
    v[0] = P { name: victim(2), age: 2 }
    let k = churn()
    setp(&mut v, 0, P { name: victim(3), age: 3 })
    let taken: P = takep(&mut v)
    Vec::push(v, P { name: victim(4), age: 4 })
    println(w[0].name + " " + v[0].name + " " + taken.name)
    return 0
}
"#;

#[test]
fn sb98_a_vec_of_structs_owns_its_elements_across_copies_stores_and_pops() {
    let h = Harness::new();
    h.differential_row(
        "SB-105",
        SB105_A_VEC_OF_STRUCTS_ACROSS_COPIES_STORES_AND_POPS,
        "ab1 ab4 ab3\n",
        (95, 95),
    );
    h.assert_legs(18);
}

const SB106_A_CSV_ROW_PARSED_INTO_A_STRUCT: &str = r#"
needs std.str
struct Person { name: string, city: string }
fn parse(line: string) -> Person {
    let f: Vec<string> = str.split(line, ",")
    return Person { name: f[0], city: f[1] }
}
fn main() -> i64 {
    let mut total: i64 = 0
    for i in 0..500 {
        let p: Person = parse("alice{i},paris,{i}")
        total = total + p.name.len + p.city.len
    }
    println("{total}")
    return 0
}
"#;

#[test]
fn sb99_a_csv_row_parsed_into_a_struct_releases_every_field() {
    let h = Harness::new();
    h.differential_row(
        "SB-106",
        SB106_A_CSV_ROW_PARSED_INTO_A_STRUCT,
        "6390\n",
        (4500, 4500),
    );
    h.assert_legs(18);
}

const SB107_A_NESTED_STRUCT: &str = r#"
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

struct A { s: string }
struct B { a: A, t: string }
fn make(i: i64) -> B { return B { a: A { s: victim(i) }, t: victim(i + 1) } }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 500 {
        let b: B = make(i)
        let c: B = b
        let a2: A = c.a
        n = n + a2.s.len + b.t.len
        i = i + 1
    }
    let k = churn()
    println("{n}")
    return 0
}
"#;

#[test]
fn sb100_a_struct_inside_a_struct_releases_the_strings_of_both() {
    let h = Harness::new();
    h.differential_row("SB-107", SB107_A_NESTED_STRUCT, "4782\n", (2081, 2081));
    h.assert_legs(18);
}

const SB108_A_GENERIC_INSTANTIATED_WITH_A_STRUCT: &str = r#"
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

struct P { name: string, age: i64 }
fn keep<T>(x: T) -> T { return x }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 500 {
        let p: P = keep(P { name: victim(i), age: i })
        n = n + p.name.len
        i = i + 1
    }
    let k = churn()
    println("{n}")
    return 0
}
"#;

#[test]
fn sb101_a_generic_instantiated_with_a_carrier_counts_it_like_a_string() {
    let h = Harness::new();
    h.differential_row(
        "SB-108",
        SB108_A_GENERIC_INSTANTIATED_WITH_A_STRUCT,
        "2390\n",
        (1081, 1081),
    );
    h.assert_legs(18);
}

const SB109_A_FILLED_ARRAY_LITERAL_PUSHED_AND_STORED: &str = r#"
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

struct S { a: [string; 3], k: i64 }
fn main() -> i64 {
    let mut v: Vec<[string; 3]> = Vec::new()
    let mut s: S = S { a: [victim(0); 3], k: 0 }
    let mut i: i64 = 1
    while i < 100 {
        Vec::push(v, [victim(i); 3])
        s.a = [victim(i + 1); 3]
        i = i + 1
    }
    let k = churn()
    println(v[0][0] + s.a[2] + "{Vec::len(v)}")
    return 0
}
"#;

#[test]
fn sb102_a_filled_array_literal_is_fresh_like_an_element_by_element_one() {
    let h = Harness::new();
    h.differential_row(
        "SB-109",
        SB109_A_FILLED_ARRAY_LITERAL_PUSHED_AND_STORED,
        "ab1ab10099\n",
        (483, 483),
    );
    h.assert_legs(18);
}

const SB110_A_SHARED_VEC_OF_CARRIERS_PUSHED_AND_POPPED: &str = r#"
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

struct P { name: string, age: i64 }
fn main() -> i64 {
    let mut v: Vec<P> = Vec::new()
    Vec::push(v, P { name: victim(1), age: 1 })
    let w: Vec<P> = v
    Vec::push(v, P { name: victim(2), age: 2 })
    let taken: P = Vec::pop(v)
    let mut u: Vec<[string; 2]> = Vec::new()
    Vec::push(u, [victim(3), victim(4)])
    let t: Vec<[string; 2]> = u
    Vec::push(u, [victim(5), victim(6)])
    let k = churn()
    println(w[0].name + taken.name + t[0][1] + u[1][0] + "{Vec::len(v)} {Vec::len(t)}")
    return 0
}
"#;

#[test]
fn sb103_a_push_or_a_pop_on_a_shared_vec_of_carriers_retains_what_it_copies() {
    let h = Harness::new();
    h.differential_row(
        "SB-110",
        SB110_A_SHARED_VEC_OF_CARRIERS_PUSHED_AND_POPPED,
        "ab1ab2ab4ab51 1\n",
        (105, 105),
    );
    h.assert_legs(18);
}

const SB111_A_CARRIER_THAT_HOLDS_A_STRING_AND_AN_RC: &str = r#"
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
struct Q { r: Rc<i64>, name: string }

fn main() -> i64 {
    let q: Q = Q { r: Rc::new(5), name: victim(1) }
    let q2: Q = q
    let k = churn()
    println(q2.name + q.name + "{Rc::get(q2.r) + Rc::get(q.r)}")
    return 0
}
"#;

#[test]
fn sb104_a_copy_of_a_carrier_that_also_holds_an_rc_keeps_both_shares() {
    let h = Harness::new();
    h.differential_row(
        "SB-111",
        SB111_A_CARRIER_THAT_HOLDS_A_STRING_AND_AN_RC,
        "ab1ab110\n",
        (87, 87),
    );
    h.assert_legs(18);
}

const SB112_A_FIELD_READ_BESIDE_A_CALL_THAT_REPLACES_IT: &str = r#"
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

struct P { name: string, age: i64 }
fn clob(p: &mut P) -> i64 {
    (*p).name = victim(1)
    let k = churn()
    return 1
}
fn show(s: string, n: i64) -> string { return s }
fn main() -> i64 {
    let mut p: P = P { name: victim(1), age: 0 }
    let s: string = show(p.name, clob(&mut p))
    println(s + p.name)
    return 0
}
"#;

#[test]
fn sb105_a_field_read_is_pinned_while_a_sibling_replaces_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-112",
        SB112_A_FIELD_READ_BESIDE_A_CALL_THAT_REPLACES_IT,
        "ab1ab1\n",
        (86, 86),
    );
    h.assert_legs(18);
}

const SB113_AN_UNBOUND_ARRAY_LITERAL_HANDED_ON: &str = r#"
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

struct S { a: [string; 3], k: i64 }
fn take(a: [string; 3]) -> i64 { return a[0].len }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 200 {
        n = n + take([victim(i); 3])
        let s: S = S { a: [victim(i + 1); 3], k: i }
        n = n + s.a[1].len + s.k % 3
        i = i + 1
    }
    let k = churn()
    println("{n}")
    return 0
}
"#;

#[test]
fn sb106_an_unbound_array_literal_is_released_at_the_end_of_its_statement() {
    let h = Harness::new();
    h.differential_row(
        "SB-113",
        SB113_AN_UNBOUND_ARRAY_LITERAL_HANDED_ON,
        "1981\n",
        (881, 881),
    );
    h.assert_legs(18);
}

const SB114_A_VEC_OF_ENUMS_WITH_A_PAYLOAD: &str = r#"
enum Shape { Named(string, i64), Sized(i64), Blank }
fn victim(n: i64) -> string { return "ab" + "{n}" }
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
    let mut w: Vec<Shape> = Vec::new()
    Vec::push(w, Shape::Sized(7))
    Vec::push(w, Shape::Named(victim(1), 5))
    Vec::push(w, Shape::Blank)
    let mut j: i64 = 0
    let mut acc: string = ""
    while j < 3 {
        match w[j] {
            Shape::Named(t, k) => { acc = acc + t + "{k}" }
            Shape::Sized(k) => { acc = acc + "s{k}" }
            Shape::Blank => { acc = acc + "b" }
        }
        j = j + 1
    }
    println("{n} " + acc)
    return 0
}
"#;

#[test]
fn sb107_a_vec_of_enums_indexes_at_the_stride_its_elements_were_pushed_at() {
    let h = Harness::new();
    h.differential_row(
        "SB-114",
        SB114_A_VEC_OF_ENUMS_WITH_A_PAYLOAD,
        "112233 s7ab15b\n",
        (14, 14),
    );
    h.assert_legs(18);
}

const SB115_AN_OPTION_OF_STRING_OWNS_ITS_CHARGE: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn hold(o: Option<string>) -> i64 {
    match o {
        Option::Some(x) => { return x.len }
        Option::None => { return 0 }
    }
}
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 20 {
        let o: Option<string> = Option::Some(victim(i))
        n = n + hold(o)
        let p: Option<string> = if i % 2 == 0 { Option::Some(victim(i + 1)) } else { Option::None }
        match p {
            Option::Some(y) => { n = n + y.len }
            Option::None => { n = n + 1 }
        }
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn sb108_an_option_of_string_releases_its_charge_at_the_end_of_its_scope() {
    let h = Harness::new();
    h.differential_row(
        "SB-115",
        SB115_AN_OPTION_OF_STRING_OWNS_ITS_CHARGE,
        "115\n",
        (60, 60),
    );
    h.assert_legs(18);
}

const SB116_A_USER_ENUM_WITH_TWO_CARRYING_VARIANTS: &str = r#"
enum Shape { Named(string, i64), Tagged(string), Blank }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn make(i: i64) -> Shape {
    if i % 3 == 0 { return Shape::Named(victim(i), i) }
    if i % 3 == 1 { return Shape::Tagged(victim(i + 1)) }
    return Shape::Blank
}
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 12 {
        let s: Shape = make(i)
        match s {
            Shape::Named(t, k) => { n = n + t.len + k }
            Shape::Tagged(t) => { n = n + t.len }
            Shape::Blank => { n = n + 1 }
        }
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn sb109_a_user_enum_counts_the_charge_of_the_variant_its_tag_names() {
    let h = Harness::new();
    h.differential_row(
        "SB-116",
        SB116_A_USER_ENUM_WITH_TWO_CARRYING_VARIANTS,
        "47\n",
        (16, 16),
    );
    h.assert_legs(18);
}

const SB117_A_VEC_OF_OPTIONS_SHARED_THEN_PUSHED: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut v: Vec<Option<string>> = Vec::new()
    Vec::push(v, Option::Some(victim(1)))
    Vec::push(v, Option::None)
    Vec::push(v, Option::Some(victim(2)))
    let w: Vec<Option<string>> = v
    Vec::push(v, Option::Some(victim(3)))
    let t: Option<string> = Vec::pop(v)
    match t {
        Option::Some(x) => { n = n + x.len }
        Option::None => { n = n + 1 }
    }
    let mut i: i64 = 0
    while i < 3 {
        match w[i] {
            Option::Some(x) => { n = n + x.len }
            Option::None => { n = n + 7 }
        }
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn sb110_a_shared_vec_of_options_dups_its_charges_when_a_push_detaches_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-117",
        SB117_A_VEC_OF_OPTIONS_SHARED_THEN_PUSHED,
        "16\n",
        (8, 8),
    );
    h.assert_legs(18);
}

const SB118_A_VARIANT_THAT_CARRIES_A_STRUCT_AND_AN_ARRAY: &str = r#"
struct P { a: string, b: i64 }
enum Box3 { Pair(P), Trio([string; 3]), Empty }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 8 {
        let b: Box3 = if i % 2 == 0 {
            Box3::Pair(P { a: victim(i), b: i })
        } else {
            Box3::Trio([victim(i), victim(i + 1), victim(i + 2)])
        }
        match b {
            Box3::Pair(p) => { n = n + p.a.len + p.b }
            Box3::Trio(t) => { n = n + t[0].len + t[2].len }
            Box3::Empty => { n = n + 1 }
        }
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn sb111_a_variant_that_carries_a_struct_or_an_array_reaches_their_leaves() {
    let h = Harness::new();
    h.differential_row(
        "SB-118",
        SB118_A_VARIANT_THAT_CARRIES_A_STRUCT_AND_AN_ARRAY,
        "48\n",
        (32, 32),
    );
    h.assert_legs(18);
}

const SB119_AN_OPTION_IN_A_GENERIC_AND_AN_ENUM_IN_A_STRUCT: &str = r#"
enum Tag2 { Some3(string), None3 }
struct Holder { t: Tag2, name: string }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn wrap<T>(x: T) -> Option<T> { return Option::Some(x) }
fn unwrap_or<T>(o: Option<T>, d: T) -> T {
    match o {
        Option::Some(x) => { return x }
        Option::None => { return d }
    }
}
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 10 {
        let o: Option<string> = wrap(victim(i))
        let s: string = unwrap_or(o, "zz")
        n = n + s.len
        let h: Holder = Holder { t: Tag2::Some3(victim(i + 1)), name: victim(i + 2) }
        let g: Holder = h
        match g.t {
            Tag2::Some3(x) => { n = n + x.len }
            Tag2::None3 => { n = n + 1 }
        }
        n = n + g.name.len
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn sb112_an_option_crossing_a_generic_and_an_enum_held_by_a_struct_own_their_charges() {
    let h = Harness::new();
    h.differential_row(
        "SB-119",
        SB119_AN_OPTION_IN_A_GENERIC_AND_AN_ENUM_IN_A_STRUCT,
        "93\n",
        (60, 60),
    );
    h.assert_legs(18);
}

const SB120_AN_UNBOUND_SCRUTINEE_AND_AN_UNBOUND_FIELD_BASE: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn get(i: i64) -> Option<string> { if i % 3 == 0 { return Option::None }  return Option::Some(victim(i)) }
struct P { a: string, b: i64 }
fn mk(i: i64) -> P { return P { a: victim(i), b: i } }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 9 {
        match get(i) {
            Option::Some(v) => { n = n + v.len }
            Option::None => { n = n + 1 }
        }
        n = n + mk(i).a.len
        match Option::Some(victim(i + 1)) {
            Option::Some(w) => { n = n + w.len }
            Option::None => { n = n + 1 }
        }
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn sb113_a_fresh_aggregate_no_binding_holds_is_released_when_the_statement_ends() {
    let h = Harness::new();
    h.differential_row(
        "SB-120",
        SB120_AN_UNBOUND_SCRUTINEE_AND_AN_UNBOUND_FIELD_BASE,
        "75\n",
        (48, 48),
    );
    h.assert_legs(18);
}

const SB121_A_QUESTION_MARK_ON_A_RESULT_THAT_CARRIES_A_STRING: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn get(i: i64) -> Result<string, i64> {
    if i % 4 == 3 { return Result::Err(i) }
    return Result::Ok(victim(i))
}
fn chain(i: i64) -> Result<string, i64> {
    let s: string = get(i)?
    return Result::Ok(s + "!")
}
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 8 {
        match chain(i) {
            Result::Ok(s) => { n = n + s.len }
            Result::Err(e) => { n = n + e % 5 }
        }
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn sb114_the_question_mark_releases_the_result_it_unwrapped() {
    let h = Harness::new();
    h.differential_row(
        "SB-121",
        SB121_A_QUESTION_MARK_ON_A_RESULT_THAT_CARRIES_A_STRING,
        "29\n",
        (18, 18),
    );
    h.assert_legs(18);
}

const SB122_A_REASSIGNED_GLOBAL_RELEASES_WHAT_IT_HELD: &str = r#"
struct P { a: string, b: i64 }
let mut g: string = "g0"
let mut h: P = P { a: "h0", b: 0 }
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 6 {
        g = victim(i)
        h = P { a: victim(i + 1), b: i }
        n = n + g.len + h.a.len
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn sb115_a_reassigned_global_releases_the_value_it_replaces() {
    let h = Harness::new();
    h.differential_row(
        "SB-122",
        SB122_A_REASSIGNED_GLOBAL_RELEASES_WHAT_IT_HELD,
        "36\n",
        (24, 22),
    );
    h.assert_legs(18);
}

const SB123_A_GLOBAL_READ_HELD_WHILE_A_SIBLING_REPLACES_IT: &str = r#"
let mut g: string = "init"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn churn() -> Vec<string> {
    let mut keep: Vec<string> = Vec::new()
    let mut i: i64 = 10
    while i < 40 {
        Vec::push(keep, "Q" + "{i}")
        i = i + 1
    }
    return keep
}
fn setg(n: i64) -> i64 {
    g = victim(n)
    let k = churn()
    return 1
}
fn keeps(a: string, n: i64) -> string { return a }
fn keeps_gen<T>(a: T, n: i64) -> T { return a }
fn main() -> i64 {
    g = victim(1)
    let t: string = keeps(g, setg(2))
    let k1 = churn()
    let u: string = keeps_gen(g, setg(3))
    let k2 = churn()
    println(t + u)
    return 0
}
"#;

#[test]
fn sb116_a_global_read_is_held_while_a_sibling_call_replaces_it() {
    let h = Harness::new();
    h.differential_row(
        "SB-123",
        SB123_A_GLOBAL_READ_HELD_WHILE_A_SIBLING_REPLACES_IT,
        "ab1ab2\n",
        (251, 250),
    );
    h.assert_legs(18);
}

const SB124_A_ZOMBIE_ARM_IS_RELEASED_ONCE: &str = r#"
fn victim(n: i64) -> string { return "ab" + "{n}" }
fn pick<T>(a: T, b: T) -> T {
    let mut i: i64 = 0
    let mut r: T = b
    while i < 6 {
        i = i + 1
        r = if i % 2 == 0 {
            let k: string = "x" + "{i}"
            a
        } else { b }
        if i == 3 { continue }
        if i == 5 { break }
    }
    return r
}
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 4 {
        let a: string = victim(i)
        let b: string = victim(i + 1)
        let r: string = pick(a, b)
        n = n + r.len
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn sb117_a_local_kept_open_past_its_block_is_released_once() {
    let h = Harness::new();
    h.differential_row(
        "SB-124",
        SB124_A_ZOMBIE_ARM_IS_RELEASED_ONCE,
        "12\n",
        (32, 32),
    );
    h.assert_legs(18);
}

const SB125_A_CLOSURE_HANDS_OUT_A_SHARE_OF_ITS_CAPTURE: &str = r#"
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

fn mk(n: i64) -> Vec<string> {
    let mut v: Vec<string> = Vec::new()
    Vec::push(v, victim(n))
    return v
}
fn main() -> i64 {
    let v: Vec<string> = mk(1)
    let g = fn() -> Vec<string> { return v }
    let w: Vec<string> = g()
    let x: Vec<string> = g()
    println("{Vec::len(g())}")
    let k = churn()
    println(w[0] + x[0])
    println(v[0])
    return 0
}
"#;

#[test]
fn sb118_a_closure_that_returns_its_captured_vec_hands_out_a_share() {
    let h = Harness::new();
    h.differential_row(
        "SB-125",
        SB125_A_CLOSURE_HANDS_OUT_A_SHARE_OF_ITS_CAPTURE,
        "1\nab1ab1\nab1\n",
        (86, 83),
    );
    h.assert_legs(18);
}

const SB126_A_SORT_THROUGH_A_MUTABLE_SLICE_KEEPS_ONE_SHARE: &str = r#"
needs std.sort

fn victim(n: i64) -> string { return "ab" + "{n}" }
fn main() -> i64 {
    let mut n: i64 = 0
    let mut i: i64 = 0
    while i < 4 {
        let mut v: Vec<string> = Vec::new()
        Vec::push(v, victim(i + 3))
        Vec::push(v, victim(i + 1))
        Vec::push(v, victim(i + 2))
        let s = Vec::try_as_unique_mut_slice(v)
        sort.strings(s)
        n = n + s[0].len
        i = i + 1
    }
    println("{n}")
    return 0
}
"#;

#[test]
fn sb119_a_store_into_a_mutable_slice_releases_the_element_it_replaces() {
    let h = Harness::new();
    h.differential_row(
        "SB-126",
        SB126_A_SORT_THROUGH_A_MUTABLE_SLICE_KEEPS_ONE_SHARE,
        "12\n",
        (28, 28),
    );
    h.assert_legs(18);
}

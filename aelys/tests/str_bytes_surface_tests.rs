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
        if fs::write(&path, "fn main() { }\n").is_err() {
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
fn main() {
    let s: String = "éab";
    println!("{}", s.len());
    println!("{}", s.as_bytes().len());
    let b = s.as_bytes();
    println!("{}", b[0]);
    println!("{}", b[1]);
    println!("{}", b[2]);
    println!("{}", b[3]);
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
fn main() {
    let s: String = "éléphant";
    let b = s.as_bytes();
    let mut sum: i64 = 0;
    for i in 0..b.len() {
        sum = sum + (b[i] as i64);
    }
    println!("{}", b.len());
    println!("{}", sum);
}
"#;

#[test]
fn sb2_a_byte_view_of_a_multibyte_string_iterates() {
    let h = Harness::new();
    h.value_row("SB-2", SB2_ITERATION, "10\n1375\n", (0, 0));
    h.assert_legs(6);
}

const SB3_NOGC_BYTES: &str = r#"
nogc fn first_byte(s: String) -> i64 {
    let b = s.as_bytes();
    if b.len() == 0 { return -1; }
    b[0] as i64
}

fn main() {
    let s: String = "éab";
    println!("{}", first_byte(s));
}
"#;

#[test]
fn sb3_a_byte_view_allocates_nothing_so_it_is_nogc() {
    let h = Harness::new();
    h.value_row("SB-3", SB3_NOGC_BYTES, "195\n", (0, 0));
    h.assert_legs(6);
}

const SB4_SUBSTRING_BYTES: &str = r#"
fn main() {
    let s: String = "éléphant";
    let head = s[0..5].to_string();
    let tail = s[5..10].to_string();
    println!("{}", head);
    println!("{}", tail);
    println!("{}", head.len());
}
"#;

#[test]
fn sb4_substring_bytes_cuts_a_multibyte_string_on_its_boundaries() {
    let h = Harness::new();
    h.value_row("SB-4", SB4_SUBSTRING_BYTES, "élé\nphant\n5\n", (2, 2));
    h.assert_legs(6);
}

const SB5_SUBSTRING_CONCAT: &str = r#"
fn main() {
    let s: String = "hello world";
    let mut t: String = "";
    let mut i: i64 = 0;
    for c in s {
        if i < 5 { t = t + c.to_string(); }
        i = i + 1;
    }
    println!("{}", t);
}
"#;

const SB5_SUBSTRING_LIBRARY: &str = r#"
use std::str;

fn main() {
    let s: String = "hello world";
    let t = str::substring(s, 0, 5);
    println!("{}", t);
}
"#;

const SB5_SUBSTRING_BYTES: &str = r#"
fn main() {
    let s: String = "hello world";
    let t = s[0..5].to_string();
    println!("{}", t);
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
fn main() {
    let s: String = "éab";
    let t = s[0..1].to_string();
    println!("{}", t);
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
fn main() {
    let s: String = "éab";
    let t = s[0..9].to_string();
    println!("{}", t);
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
fn main() {
    let s: String = "éab";
    println!("{}", s[3]);
}
"#;

const SB8_CHARACTERS_AND_BYTES: &str = r#"
use std::str;

fn main() {
    let s: String = "éab";
    println!("{}", s.as_bytes().len());
    println!("{}", str::char_count(s));
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
use std::str;

fn main() {
    let s: String = "éab";
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < s.len() {
        let k: i64 = str::char_len(s, i);
        println!("{}", str::is_char_boundary(s, i));
        println!("{}", k);
        i = i + k;
        n = n + 1;
    }
    println!("{}", str::is_char_boundary(s, 1));
    println!("{}", str::char_len(s, 1));
    println!("{}", n);
    println!("{}", s.len());
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
fn main() {
    let s: String = "éab";
    let b = s.as_bytes();
    b[0] = 65;
}
"#;

const SB11_BYTES_OUTLIVES_ITS_STRING: &str = r#"
use std::process::ExitCode; fn main() -> ExitCode {
    let base: String = "éab";
    let mut r = base.as_bytes();
    {
        let inner: String = "ünz";
        r = inner.as_bytes();
    }
    return ExitCode::from((r[0] as i64) as u8);
}
"#;

const SB12_NO_CHARACTER_COUNT: &str = r#"
fn main() {
    let s: String = "éab";
    println!("{}", s.chars);
}
"#;

const SB13_INDEX_ASSIGN_ON_A_STRING: &str = r#"
fn main() {
    let mut s: String = "éab";
    s[0] = "x";
}
"#;

const SB14_SUBSTRING_BYTES_IN_A_NOGC_FN: &str = r#"
nogc fn cut(s: String) -> String {
    s[0..2].to_string()
}

fn main() {
    let s: String = "éab";
    println!("{}", cut(s));
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
use std::str;

fn build(src: String) -> Vec<String> {
    let mut out = Vec::new();
    let a: String = src[0..3].to_string();
    let b: String = a;
    Vec::push(out, b);
    out
}

fn main() {
    let v: Vec<String> = build("abcdefgh");
    let poison: String = "ZZZZZZZZ"[0..3].to_string();
    println!("{}", poison);
    println!("{}", v[0]);
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
use std::str;

fn build(src: String) -> Vec<String> {
    let mut out = Vec::new();
    let mut a: String = src[0..3].to_string();
    let mut b: String = src[3..6].to_string();
    a = b;
    Vec::push(out, b);
    println!("{}", a);
    out
}

fn main() {
    let v: Vec<String> = build("abcdefgh");
    let poison: String = "ZZZZZZZZ"[0..3].to_string();
    println!("{}", poison);
    println!("{}", v[0]);
}
"#;

const SB17_A_SLOT_REASSIGNED_TO_ITSELF: &str = r#"
use std::str;

fn main() {
    let mut s: String = "abcdefgh"[0..3].to_string();
    s = s;
    let poison: String = "ZZZZZZZZ"[0..3].to_string();
    println!("{}", poison);
    println!("{}", s);
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
        (2, 2),
    );
    h.assert_legs(36);
}

const SB18_TWO_SLOTS_REASSIGNED_FROM_ONE_SOURCE: &str = r#"
use std::str;

fn build(src: String) -> Vec<String> {
    let mut out = Vec::new();
    let b: String = src[2..5].to_string();
    Vec::push(out, b);
    {
        let mut a1: String = src[0..1].to_string();
        a1 = b;
        println!("{}", a1);
    }
    let poison: String = "ZZZZZZZZ"[0..3].to_string();
    println!("{}", poison);
    {
        let mut a2: String = src[1..2].to_string();
        a2 = b;
        println!("{}", a2);
    }
    let second: String = "WWWWWWWW"[0..3].to_string();
    println!("{}", second);
    println!("{}", poison);
    out
}

fn main() {
    let v: Vec<String> = build("abcdefgh");
    println!("{}", v[0]);
    println!("survived");
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
use std::str;

fn build(src: String) -> Vec<String> {
    let mut out = Vec::new();
    let mut a: String = src[0..3].to_string();
    Vec::push(out, a);
    a = src[3..6].to_string();
    println!("{}", a);
    out
}

fn main() {
    let v: Vec<String> = build("abcdefgh");
    let poison: String = "ZZZZZZZZ"[0..3].to_string();
    println!("{}", poison);
    println!("{}", v[0]);
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
use std::str;

fn touch(s: String) -> i64 {
    s.len()
}

fn main() {
    let mut out: String = "";
    let mut i: i64 = 0;
    while i < 4 {
        out = "abcdefgh"[0..3].to_string();
        i = i + 1;
    }
    println!("{}", out);
    println!("{}", touch(out));
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
use std::str;

fn build(src: String) -> Vec<String> {
    let mut out = Vec::new();
    let b: String = src[0..3].to_string();
    let mut a: String = b;
    a = src[3..6].to_string();
    Vec::push(out, a);
    Vec::push(out, b);
    out
}

fn main() {
    let v: Vec<String> = build("abcdefgh");
    let poison: String = "ZZZZZZZZ"[0..3].to_string();
    println!("{}", poison);
    println!("{}", v[0]);
    println!("{}", v[1]);
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
use std::str;

fn id(s: String) -> String {
    s
}

fn main() {
    let mut v: Vec<String> = Vec::new();
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 3 {
        n = n + (format!("{i}") + "!").len();
        if format!("{i}") == "1" { n = n + 100; }
        Vec::push(v, "k" + format!("{i}"));
        let kept: String = id("r" + format!("{i}"));
        let poison: String = "ZZZZZZZZ"[0..2].to_string();
        println!("{}", kept + poison);
        println!("row {i} of {n}");
        i = i + 1;
    }
    let poison: String = "ZZZZZZZZ"[0..2].to_string();
    println!("{}", v[0] + v[1] + v[2] + poison);
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
fn pair(a: String, b: String) -> String {
    a + "|" + b
}

fn main() {
    let x: i64 = 11;
    let y: i64 = 22;
    let f: f64 = 2.5;
    let c: char = 'é';
    let u: u8 = 200;
    let mut i: i64 = 0;
    while i < 2 {
        println!("{x}");
        println!("{}", y);
        println!("{}", f);
        println!("{}", c);
        println!("{}", true);
        println!("{}", u);
        println!("{}", pair(format!("{x}"), format!("{y}")));
        i = i + 1;
    }
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
use std::str;

fn main() {
    let outer: String = "out" + format!("{}", 7);
    let mut v: Vec<String> = Vec::new();
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 6 {
        i = i + 1;
        let s: String = "s" + format!("{i}");
        if i % 2 == 0 {
            let kept: String = "k" + format!("{i}");
            Vec::push(v, kept);
            continue;
        }
        let mut j: i64 = 0;
        while j < 4 {
            let t: String = "t" + format!("{j}");
            j = j + 1;
            if j == 3 {
                n = n + t.len();
                break;
            }
            n = n + t.len() + s.len();
        }
        if i == 5 {
            let poison: String = "ZZZZZZZZ"[0..2].to_string();
            println!("{}", s + poison);
            break;
        }
    }
    let poison: String = "ZZZZZZZZ"[0..2].to_string();
    println!("{}", outer + poison);
    println!("{}", n);
    println!("{}", v[0] + v[1]);
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
use std::str;

fn label(i: i64) -> String {
    format!("L{i}")
}

fn relabel(i: i64) -> String {
    label(i)
}

fn id(s: String) -> String {
    s
}

fn main() {
    let mut v: Vec<String> = Vec::new();
    let base: String = "b" + format!("{}", 7);
    let mut i: i64 = 0;
    while i < 3 {
        let a: String = label(i);
        let b: String = relabel(i);
        let c: String = id(base);
        let t: String = str::trim(format!(" {i} "));
        Vec::push(v, label(i + 10));
        let poison: String = "ZZZZZZZZ"[0..2].to_string();
        println!("{}", a + b + c + t + poison);
        println!("{}", label(i).len() + str::from_int(i).len());
        i = i + 1;
    }
    let poison: String = "ZZZZZZZZ"[0..2].to_string();
    println!("{}", v[0] + v[2] + base + poison);
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
fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < TURNS {
        let s: String = "row " + format!("{i}");
        n = n + s.len();
        i = i + 1;
    }
    println!("{}", n);
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
use std::str;

enum K { A, B }

fn kind(i: i64) -> K {
    if i % 3 == 2 { return K::A; }
    K::B
}

fn label(i: i64) -> String {
    format!("L{i}")
}

fn pick(p: String) -> String {
    p
}

fn main() {
    let mut hold: String = "h" + format!("{}", 0);
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 6 {
        i = i + 1;
        let w: i64 = if i == 4 {
            let t: String = str::trim(format!("  x{i}  "));
            t.len()
        } else {
            0
        };
        n = n + w;
        match kind(i) {
            K::A => {
                let a: String = "a" + format!("{i}");
                let b: String = label(i);
                println!("{}", a + b);
            }
            K::B => {
                hold = label(i + 10);
                continue;
            }
        }
    }
    let z: String = pick("Z" + format!("{}", 9));
    println!("{}", hold);
    println!("{}", z);
    println!("{}", n);
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
use std::str::split;

fn pick(p: String) -> String {
    p
}

fn junk(k: i64) -> i64 {
    let mut t: i64 = 0;
    let mut j: i64 = 0;
    while j < 8 {
        let z: String = "ZZZZZZZZ" + format!("{k}");
        t = t + z.len();
        j = j + 1;
    }
    t
}

fn main() {
    let parts: Vec<String> = split("alpha,beta,gamma", ",");
    let keep: String = "keep-" + format!("{}", 1);
    let mut last: String = "none" + format!("{}", 0);
    let mut cur: String = "cur-" + format!("{}", 2);
    for p in Vec::as_slice(parts) {
        last = p;
        cur = pick(keep);
    }
    let k: i64 = junk(3);
    for q in Vec::as_slice(parts) {
        println!("{}", q);
    }
    println!("{}", last);
    println!("{}", cur);
    println!("{}", keep);
    println!("{}", k);
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
fn main() {
    let mut i: i64 = 0;
    while i < 3 {
        fn helper(n: i64) -> i64 {
            let piece: String = "p" + format!("{n}");
            if n == 1 { break; }
            println!("{}", piece);
            n
        }
        println!("{}", helper(i));
        i = i + 1;
    }
}
"#;

const SB29_CONTINUE_IN_A_LAMBDA: &str = r#"
fn main() {
    let mut i: i64 = 0;
    while i < 3 {
        let f = move |n: i64| -> i64 {
            let piece: String = "p" + format!("{n}");
            if n == 1 { continue; }
            println!("{}", piece);
            n
        };
        println!("{}", f(i));
        i = i + 1;
    }
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
fn label(i: i64) -> String {
    format!("L{i}")
}

fn turns(k: i64) -> i64 {
    k * 2 + 1
}

fn main() {
    let mut hold: String = "h" + format!("{}", 0);
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    let last: i64 = turns(1);
    while i < last {
        let x: String = {
            let t: String = "row " + format!("{i}");
            t
        };
        n = n + x.len();
        n = n + {
            let u: String = "uu" + format!("{i}");
            u
        }.len();
        hold = {
            let w: String = "w" + format!("{i}");
            n = n + w.len();
            label(i)
        };
        println!("{}", {
            let p: String = "p" + format!("{i}");
            p
        });
        let y: String = {
            let q: String = "row" + format!("{i}");
            q
        } + ("Q" + format!("{i}"));
        println!("{}", y);
        let poison: String = "ZZZZZZZZ" + format!("{i}");
        n = n + poison.len();
        i = i + 1;
    }
    println!("{}", hold);
    println!("{}", n);
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
    if i % 2 == 0 { return K::A; }
    K::B
}

fn main() {
    let mut hold: String = "h" + format!("{}", 0);
    let tail: String = "yyyyy" + format!("{}", 1);
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 4 {
        match kind(i) {
            K::A => {
                let v: Vec<i64> = Vec::new();
                Vec::push(v, 40 + i);
                let r: Rc<i64> = Rc::new(i);
                let c: W = W { r: Rc::new(1000) };
                n = n + v[0] + Rc::get(r) + Rc::get(c.r);
            }
            K::B => {
                hold = "xx" + tail;
            }
        }
        let w: i64 = if i % 2 == 1 {
            let u: Vec<i64> = vec![i, i];
            u[0]
        } else {
            7
        };
        n = n + w;
        println!("{}", {
            let b: Vec<i64> = vec![i, 2 * i];
            b[1]
        });
        if i == 1 {
            let q: i64 = 5;
            n = n + q;
        }
        i = i + 1;
    }
    let p: String = "ZZZZZZZZ" + tail;
    println!("{}", hold);
    println!("{}", p);
    println!("{}", n);
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

fn main() {
    let w: W = W { r: Rc::new(7) };
    fn helper(a: W, c: W, d: W) -> i64 {
        Rc::get(d.r) + 1
    }
    println!("{}", helper(w, w, w));
    let f = move |a: W, c: W, d: W| -> i64 {
        Rc::get(d.r) + 2
    };
    println!("{}", f(w, w, w));
    let p: Rc<i64> = Rc::new(99);
    println!("{}", Rc::get(w.r));
    println!("{}", Rc::get(p));
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
    Rc::get(x) * 10 + Rc::get(y)
}

fn main() {
    println!("{}", add({
        let s: W = W { r: Rc::new(1) };
        s.r
    }, {
        let t: W = W { r: Rc::new(2) };
        t.r
    }));
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
        1
    }
    let k: i64 = inner();
    let y: T = x;
    y
}

fn main() {
    println!("{}", outer(41) + 1);
}
"#;

#[test]
fn sb29_a_nested_function_leaves_the_type_parameters_of_its_generic_parent_in_scope() {
    let h = Harness::new();
    h.differential_row("SB-34", SB34_NESTED_FN_IN_A_GENERIC, "42\n", (0, 0));
    h.assert_legs(18);
}

const SB35_VIEW_INTO_A_DEEPER_RC: &str = r#"
fn main() {
    let s: &[i64] = {
        let a: Rc<[i64; 2]> = Rc::new([1, 2]);
        Rc::get(a)[0..2]
    };
    let p: Rc<[i64; 2]> = Rc::new([9, 9]);
    println!("{}", s[0] * 1000 + Rc::get(p)[0]);
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
    w
}

fn getw(w: W) -> i64 {
    Rc::get(w.r)
}

fn through<T>(k: i64, mk: fn(i64) -> W, f: fn(W) -> T, g: fn(T) -> i64) -> i64 {
    let r: T = {
        let t: W = mk(k);
        f(t)
    };
    let p: Rc<i64> = Rc::new(9);
    g(r) * 1000 + Rc::get(p)
}

fn mkw(k: i64) -> W {
    W { r: Rc::new(k) }
}

fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 3 {
        n = n + through(i, mkw, idw, getw);
        i = i + 1;
    }
    println!("{}", n);
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
    r
}

fn read(o: Option<&i64>) -> i64 {
    match o {
        Some(r) => *r,
        None => 0,
    }
}

fn through<T>(k: i64, f: fn(&i64) -> T, g: fn(Option<T>) -> i64) -> i64 {
    let r: Option<T> = {
        let a: Rc<i64> = Rc::new(k);
        Some(f(&Rc::get(a)))
    };
    let p: Rc<i64> = Rc::new(9);
    g(r) * 1000 + Rc::get(p)
}

fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 3 {
        n = n + through(i, ident, read);
        i = i + 1;
    }
    println!("{}", n);
}
"#;

#[test]
fn sb32_a_block_whose_result_wraps_a_reference_type_argument_is_refused() {
    let h = Harness::new();
    h.rejects(
        "SB-37",
        SB37_GENERIC_ENUM_BLOCK_RESULT,
        "E0714",
        "the call to `through` in `main` instantiates it with `&i64`",
    );
    h.assert_legs(1);
}

const SB38_BINDINGS_THAT_OWN: &str = r#"
use std::str;

fn mk(i: i64) -> String {
    let s: String = "mk" + format!("{i}");
    s
}

fn pick(p: String, i: i64) -> String {
    let mut acc: String = p;
    acc = acc + format!("{i}");
    acc
}

fn main() {
    let mut n: i64 = 0;
    let mut hold: String = "h" + format!("{}", 0);
    let mut i: i64 = 0;
    while i < 3 {
        let a: String = "a" + format!("{i}");
        let b: String = a;
        hold = b;
        let t: String = mk(i);
        let u: String = pick(t, i);
        let r: String = str::repeat("r", 2);
        for c in a {
            if c == 'a' { n = n + 1; }
        }
        let mut m: String = "m" + format!("{i}");
        for c in m {
            if c == 'm' { n = n + 1; }
        }
        m = "q" + format!("{i}");
        for c in "w" + format!("{i}") {
            if c == 'w' { n = n + 1; }
        }
        let poison: String = "ZZZZZZZZ" + format!("{i}");
        println!("{}", a + b + t + u + r + m);
        n = n + poison.len();
        i = i + 1;
    }
    let late: String = "YYYYYYYY" + format!("{n}");
    println!("{}", hold);
    println!("{}", n + late.len());
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
fn first_vowel(s: String) -> i64 {
    let mut n: i64 = 0;
    for c in s + format!("{n}") {
        if c == 'e' { return n; }
        n = n + 1;
    }
    -1
}

fn main() {
    let mut total: i64 = 0;
    let mut i: i64 = 0;
    while i < 3 {
        let mut m: String = "abc" + format!("{i}");
        for c in m {
            if c == 'b' { continue; }
            if c == 'c' { break; }
            total = total + 1;
        }
        m = "zz" + format!("{i}");
        for c in "xy" + format!("{i}") {
            if c == 'y' { break; }
            total = total + 10;
        }
        total = total + first_vowel("hello");
        let poison: String = "ZZZZZZZZ" + format!("{i}");
        println!("{}", m + poison);
        i = i + 1;
    }
    println!("{}", total);
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
    if i % 2 == 0 { return K::A; }
    K::B
}

fn pick(i: i64) -> String {
    if i % 2 == 0 { "even" + format!("{i}") } else { "odd" + format!("{i}") }
}

fn width(s: String) -> i64 {
    s.len()
}

fn measured(i: i64) -> i64 {
    (if i % 2 == 0 { "wide" + format!("{i}") } else { "w" + format!("{i}") }).len() + width("x")
}

fn main() {
    let keep: String = "k" + format!("{}", 7);
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 6 {
        i = i + 1;
        let s: String = if i % 3 == 0 { "row " + format!("{i}") } else { keep };
        let t: String = match kind(i) {
            K::A => "a" + format!("{i}"),
            K::B => {
                let u: String = "b" + format!("{i}");
                u
            }
        };
        if i > 4 { continue; }
        let poison: String = "ZZZZZZZZ" + format!("{i}");
        println!("{}", s + t + pick(i) + poison);
        n = n + measured(i) + (if i % 2 == 0 { s } else { t }).len();
    }
    println!("{}", keep);
    println!("{}", n);
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
fn tail_of(p: String, i: i64) -> String {
    {
        let k: i64 = i;
        p
    }
}

fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 6 {
        let x: String = "x" + format!("{i}");
        let y: String = "yy" + format!("{i}");
        n = n + {
            let k: i64 = i;
            x
        }.len() + (if i > 2 { 1 } else { 0 });
        println!("{}", (if i % 2 == 0 { x } else { y }) + (if i % 3 == 0 { y } else { x }));
        if (if i % 2 == 0 { x } else { y }).len() > 2 {
            n = n + 100;
        }
        let w: i64 = if i > 3 {
            ({
                let k: i64 = i;
                y
            }).len()
        } else {
            0
        };
        let t: String = tail_of("lit", i);
        let poison: String = "ZZZZZZZZ" + format!("{i}");
        n = n + w + t.len() + poison.len();
        i = i + 1;
    }
    println!("{}", n);
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
    if i % 2 == 0 { return K::A; }
    K::B
}

fn main() {
    let mut i: i64 = 0;
    let mut x: String = "xx" + format!("{}", 0);
    let mut y: String = "y" + format!("{}", 0);
    let keep: String = "keep" + format!("{}", 0);
    let mut n: i64 = 0;
    while i < 10 && match kind(i) {
        K::A => (if i % 4 == 0 { x } else { y }).len() > 1,
        K::B => ({
            let k: i64 = i;
            if k > 2 { x } else { keep }
        }).len() > 1 && (if i > 7 { "zz" + format!("{i}") } else { y }).len() > 0,
    } {
        let before: String = x;
        x = "xx" + format!("{}", i + 1);
        y = "y" + format!("{}", i + 1);
        let poison: String = "PPPPPPPPPPPPPPPPPPPPPPPPPP" + format!("{i}");
        let poison2: String = "QQQQQQQQQQQQQQQQQQQQQQQQQQ" + format!("{i}");
        n = n + poison.len() + poison2.len();
        println!("{}", before + " " + x + " " + y);
        if (if i % 3 == 0 { x } else { before }).len() > 3 && ({
            let k: i64 = i;
            y
        }).len() > 1 {
            n = n + 1;
        }
        i = i + 1;
    }
    println!("{}", keep);
    println!("{}", n);
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
    if i % 2 == 0 { return K::A; }
    K::B
}

fn main() {
    let mut i: i64 = 0;
    let mut hits: i64 = 0;
    let keep: String = "keep" + format!("{}", 0);
    while i < 8 {
        i = i + 1;
        let x: String = "xx" + format!("{i}");
        let y: String = "y" + format!("{i}");
        let a: bool = (if i % 2 == 0 { x } else { y }).len() == 3 && ({
            let k: i64 = i;
            x
        }).len() > 2;
        let b: bool = i > 3 || (if i % 3 == 0 { "ab" + format!("{i}") } else { y }).len() == 2 && match kind(i) {
            K::A => (if i > 1 { x } else { keep }).len() > 0,
            K::B => ({
                let q: String = "q" + format!("{i}");
                q
            }).len() == 2,
        };
        let s: String = (if a { x } else { y }) + (if b && (if i > 5 { keep } else { x }).len() > 3 { "!" } else { "?" });
        let poison: String = "PPPPPPPPPPPPPPPPPPPPPPPPPP" + format!("{i}");
        let poison2: String = "QQQQQQQQQQQQQQQQQQQQQQQQQQ" + format!("{i}");
        if a { hits = hits + 1; }
        if b { hits = hits + 10; }
        println!("{}", s + " " + format!("{}", poison.len() + poison2.len()));
    }
    println!("{}", keep);
    println!("{}", hits);
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
    if i % 2 == 0 { return Result::Err(i); }
    Result::Ok(i)
}

fn stepf(i: i64) -> Result<i64, i64> {
    let x: String = "xx" + format!("{i}");
    let n: i64 = (if i % 3 == 0 { x } else { "f" + format!("{i}") }).len() + probe(i)?;
    Result::Ok(n)
}

fn main() {
    let mut i: i64 = 0;
    let mut total: i64 = 0;
    while i < 12 {
        let r: i64 = match stepf(i) {
            Result::Ok(v) => v,
            Result::Err(e) => 0 - 1,
        };
        total = total + r;
        i = i + 1;
    }
    println!("{}", total);
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
    if i % 3 == 0 { return K::A; }
    if i % 3 == 1 { return K::B; }
    K::C
}

fn probe(i: i64) -> Result<i64, i64> {
    if i % 4 == 3 { return Result::Err(i); }
    Result::Ok(i)
}

fn ret_in_inner_loop(i: i64, p: String) -> i64 {
    let x: String = "rx" + format!("{i}");
    let n: i64 = (if i % 2 == 0 { x } else { "f" + format!("{i}") }).len() + {
        let mut k: i64 = 0;
        while k < 5 {
            let q: i64 = (if k % 2 == 0 { p } else { x }).len() + match kind(k) {
                K::A => {
                    if k == i { return k * 1000 + (if k > 2 { x } else { p }).len(); }
                    1
                },
                K::B => {
                    if k > i { break; }
                    2
                },
                K::C => {
                    if k == 2 { k = k + 1;
                        continue; }
                    3
                }
            };
            k = k + q - q + 1;
        }
        k
    };
    n
}

fn try_in_cond(i: i64, p: String) -> Result<i64, i64> {
    let mut k: i64 = 0;
    let mut acc: i64 = 0;
    while k < 6 && (if k % 2 == 0 { p } else { "c" + format!("{k}") }).len() > 0 && probe(i + k)? >= 0 {
        acc = acc + (match kind(k) {
            K::A => if probe(k)? > 1 { p } else { "a" + format!("{k}") },
            K::B => p,
            K::C => ({
                let z: i64 = probe(k + 1)?;
                "cc" + format!("{z}")
            }),
        }).len();
        k = k + 1;
    }
    Result::Ok(acc)
}

fn cont_in_bounds(i: i64, p: String) -> i64 {
    let mut outer: i64 = 0;
    let mut acc: i64 = 0;
    while outer < 4 {
        outer = outer + 1;
        let t: i64 = (if outer % 2 == 0 { p } else { "o" + format!("{outer}") }).len() + {
            let mut s: i64 = 0;
            for j in 0..({
                if outer == 3 { continue; }
                (if j0(outer) { p } else { "bb" + format!("{outer}") }).len()
            }) {
                s = s + j;
            }
            s
        };
        while { if outer == 4 { break; }; (if outer > 1 { p } else { "w" }).len() } > 100 {
            acc = acc + 1;
        }
        acc = acc + t;
    }
    acc
}

fn j0(n: i64) -> bool {
    n % 2 == 0
}

fn main() {
    let mut i: i64 = 0;
    let mut total: i64 = 0;
    while i < 6 {
        let p: String = "pp" + format!("{i}");
        let a: i64 = ret_in_inner_loop(i, "lit");
        let b: i64 = match try_in_cond(i, "lit2") {
            Result::Ok(v) => v,
            Result::Err(e) => 0 - e,
        };
        let c: i64 = cont_in_bounds(i, "lit3");
        let poison: String = "PPPPPPPPPPPPPPPPPPPPPPPPPP" + format!("{i}");
        let poison2: String = "QQQQQQQQQQQQQQQQQQQQQQQQQQ" + format!("{i}");
        total = total + a + b + c + poison.len() + poison2.len() + p.len();
        println!("{a} {b} {c}");
        i = i + 1;
    }
    println!("{}", total);
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
fn main() {
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 6 {
        let x: String = "xx" + format!("{i}");
        let y: String = "y" + format!("{i}");
        n = n + (if i % 2 == 0 { x } else { y }).len() + {
            if i > 2 { n = n + 1; }
            0
        };
        i = i + 1;
    }
    println!("{}", n);
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
fn add(v: &mut Vec<String>, x: String) -> String {
    let s: String = x + "!";
    Vec::push(*v, s);
    s
}
fn main() {
    let mut names: Vec<String> = Vec::new();
    let mut i: i64 = 0;
    while i < 3 {
        println!("{}", add(&mut names, "n" + format!("{i}")));
        i = i + 1;
    }
    let p: String = "QQ" + format!("{}", 8);
    println!("{}", names[0]);
    println!("{}", names[2]);
    println!("{}", p);
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
static mut G: String = "init";
fn f(s: String, c: bool) -> i64 {
    let y: String = if c {
        let x: String = s + "!";
        G = x;
        x
    } else {
        "n"
    };
    println!("{}", y);
    y.len()
}
fn main() {
    let n: i64 = f("abc", true);
    let p: String = "QQ" + format!("{}", 8);
    println!("{}", G);
    println!("{}", p);
    println!("{}", n);
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
fn main() {
    let s: String = "ab" + format!("{}", 1);
    let mut n: i64 = 0;
    for c in {
        let x: String = s + "!";
        x
    } {
        let p: String = "QQ" + format!("{}", 8);
        print!("{}", c);
        n = n + p.len();
    }
    println!("");
    println!("{n}");
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
use std::str;

fn peek(s: &String) -> i64 {
    (*s).len()
}

fn build(src: String) -> i64 {
    let mut a: String = src[0..3].to_string();
    let n: i64 = peek(&a);
    a = src[3..6].to_string();
    println!("{}", a);
    n
}

fn main() {
    let n: i64 = build("abcdefgh");
    let poison: String = "ZZZZZZZZ"[0..3].to_string();
    println!("{}", poison);
    println!("{n}");
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
use std::str;

fn touch(s: &String) -> i64 {
    (*s).len()
}

fn main() {
    let mut out: String = "";
    let mut i: i64 = 0;
    while i < 4 {
        out = "abcdefgh"[0..3].to_string();
        i = i + 1;
    }
    println!("{}", out);
    println!("{}", touch(&out));
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
struct P { name: String }
fn mk(s: String) -> P { P { name: s } }
fn wrap(s: String) -> Option<String> { Some(s) }
fn arr(s: String) -> [String; 2] { [s, "c"] }
fn fill(s: String) -> [String; 3] { [s; 3] }
fn main() {
    let p: P = mk("ab" + format!("{}", 1));
    let o: Option<String> = wrap("ab" + format!("{}", 2));
    let a: [String; 2] = arr("ab" + format!("{}", 3));
    let b: [String; 3] = fill("ab" + format!("{}", 4));
    let q: String = "QQ" + format!("{}", 8);
    let r: String = "QQ" + format!("{}", 9);
    println!("{}", p.name);
    match o {
        Some(x) => println!("{}", x),
        None => println!("none"),
    }
    println!("{}", a[0]);
    println!("{}", b[0]);
    println!("{}", b[2]);
    println!("{}", q);
    println!("{}", r);
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
struct P { name: String }
static mut G: String = "init";
fn put(v: &mut Vec<String>, s: String) { Vec::push(*v, s); }
fn setn(p: &mut P, s: String) { (*p).name = s; }
fn seti(a: &mut [String], s: String) { a[0] = s; }
fn setd(r: &mut String, s: String) { *r = s; }
fn setg(s: String) { G = s; }
fn main() {
    let mut v: Vec<String> = Vec::new();
    let mut p: P = P { name: "x" };
    let mut a: [String; 2] = ["x", "y"];
    let mut d: String = "x";
    put(&mut v, "ab" + format!("{}", 1));
    setn(&mut p, "ab" + format!("{}", 2));
    seti(a[..], "ab" + format!("{}", 3));
    setd(&mut d, "ab" + format!("{}", 4));
    setg("ab" + format!("{}", 5));
    let q: String = "QQ" + format!("{}", 8);
    let r: String = "QQ" + format!("{}", 9);
    println!("{}", v[0]);
    println!("{}", p.name);
    println!("{}", a[0]);
    println!("{}", d);
    println!("{}", G);
    println!("{}", q);
    println!("{}", r);
}
"#;

#[test]
fn sb48_a_parameter_stored_through_a_reference_or_into_a_global_takes_a_share_of_its_own() {
    let h = Harness::new();
    h.differential_row(
        "SB-54",
        SB54_A_PARAMETER_STORED_THROUGH_A_REFERENCE_OR_INTO_A_GLOBAL,
        "ab1\nab2\nab3\nab4\nab5\nQQ8\nQQ9\n",
        (15, 14),
    );
    h.assert_legs(18);
}

const SB55_A_STRING_KEPT_BY_AN_RC_OR_A_CLOSURE: &str = r#"
fn mkf(s: String) -> fn() -> String { move || -> String { s } }
fn mkg() -> fn(String) -> String {
    let mut held: String = "x";
    move |u: String| -> String {
        let old: String = held;
        held = u;
        old
    }
}
fn main() {
    let mut t: String = "ab" + format!("{}", 1);
    let b: Rc<String> = Rc::new(t);
    t = "zz";
    let f = mkf("ab" + format!("{}", 2));
    let g = mkg();
    let first: String = g("ab" + format!("{}", 3));
    let q: String = "QQ" + format!("{}", 8);
    let r: String = "QQ" + format!("{}", 9);
    println!("{}", Rc::get(b));
    println!("{}", f());
    println!("{}", first);
    println!("{}", g("zz"));
    println!("{}", q);
    println!("{}", r);
}
"#;

#[test]
fn sb49_a_string_kept_by_an_rc_or_a_closure_takes_a_share_of_its_own() {
    let h = Harness::new();
    h.differential_row(
        "SB-55",
        SB55_A_STRING_KEPT_BY_AN_RC_OR_A_CLOSURE,
        "ab1\nab2\nx\nab3\nQQ8\nQQ9\n",
        (13, 13),
    );
    h.assert_legs(18);
}

const SB56_A_PARAMETER_HANDED_TO_A_GENERIC_CALLEE: &str = r#"
fn keep<T>(x: T) -> Option<T> { Some(x) }
fn through(s: String) -> Option<String> { keep(s) }
fn main() {
    let a: Option<String> = through("ab" + format!("{}", 1));
    let s: String = "ab" + format!("{}", 2);
    let b: Option<String> = through(s);
    let q: String = "QQ" + format!("{}", 8);
    let r: String = "QQ" + format!("{}", 9);
    match a {
        Some(x) => println!("{}", x),
        None => println!("none"),
    }
    match b {
        Some(x) => println!("{}", x),
        None => println!("none"),
    }
    println!("{}", s);
    println!("{}", q);
    println!("{}", r);
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
struct P { name: String }
fn consume(s: String) -> i64 { s.len() }
fn mk(s: String) -> P { P { name: s } }
fn main() {
    let mut total: i64 = 0;
    let mut i: i64 = 0;
    while i < 4 {
        total = total + consume("x" + format!("{i}"));
        let s: String = "y" + format!("{i}");
        total = total + consume(s);
        let f = consume;
        total = total + f(s);
        i = i + 1;
    }
    let m = mk;
    let p: P = m("ab" + format!("{}", 1));
    let q: String = "QQ" + format!("{}", 8);
    println!("{total}");
    println!("{}", p.name);
    println!("{}", q);
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
fn mkv(s: String) -> Vec<String> { vec![s, "y"] }
fn mka(s: String) -> [String; 2] {
    let a: [String; 2] = [s, "y"];
    a
}
fn main() {
    let v: Vec<String> = mkv("ab" + format!("{}", 1));
    let a: [String; 2] = mka("ab" + format!("{}", 2));
    let q: String = "QQ" + format!("{}", 8);
    let r: String = "QQ" + format!("{}", 9);
    println!("{}", v[0]);
    println!("{}", a[0]);
    println!("{}", q);
    println!("{}", r);
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
use std::result;

fn keep(s: String) -> Result<String, i64> { result::ok(s) }

fn main() {
    let a: Result<String, i64> = keep("ab" + format!("{}", 1));
    let b: Result<String, i64> = {
        let t: String = "ab" + format!("{}", 2);
        keep(t)
    };
    let q: String = "QQ" + format!("{}", 8);
    let r: String = "QQ" + format!("{}", 9);
    match a {
        Ok(x) => println!("{}", x),
        Err(_) => println!("e"),
    }
    match b {
        Ok(x) => println!("{}", x),
        Err(_) => println!("e"),
    }
    println!("{}", q);
    println!("{}", r);
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
fn got(o: Option<String>) -> String {
    match o {
        Some(x) => { return x; }
        None => { return "none"; }
    }
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, "ab" + format!("{}", 1));
    Vec::push(v, "ab" + format!("{}", 2));
    Vec::push(v, "ab" + format!("{}", 3));
    let w: Vec<String> = v;
    v[0] = "ab" + format!("{}", 9);
    v[1] = v[1];
    v[2] = v[1];
    let a: String = got(Vec::pop(v));
    println!("{}", got(Vec::pop(v)));
    let q: String = "QQ" + format!("{}", 8);
    let r: String = "QQ" + format!("{}", 7);
    println!("{}", a);
    println!("{}", v[0]);
    println!("{}", w[0]);
    println!("{}", w[1]);
    println!("{}", w[2]);
    println!("{}", q);
    println!("{}", r);
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
fn edit(mut v: Vec<String>) -> String {
    v[0] = "ab" + format!("{}", 7);
    v[0]
}
fn poke(v: &mut Vec<String>) {
    (*v)[1] = "ab" + format!("{}", 8);
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, "ab" + format!("{}", 1));
    Vec::push(v, "ab" + format!("{}", 2));
    let w: Vec<String> = v;
    let e: String = edit(v);
    poke(&mut v);
    let keep: String = v[0];
    v = Vec::new();
    Vec::push(v, "ab" + format!("{}", 3));
    let q: String = "QQ" + format!("{}", 5);
    let r: String = "QQ" + format!("{}", 6);
    println!("{}", e);
    println!("{}", keep);
    println!("{}", v[0]);
    println!("{}", w[0]);
    println!("{}", w[1]);
    println!("{}", q);
    println!("{}", r);
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
use std::str;

fn mk(n: i64) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, "k" + format!("{n}"));
    Vec::push(v, "m" + format!("{n}"));
    v
}
fn count(v: Vec<String>) -> i64 { Vec::len(v) }
fn main() {
    let mut total: i64 = 0;
    let mut i: i64 = 0;
    while i < 3 {
        let line: String = format!("a{i},bb{i},c{i}");
        total = total + str::char_count(str::split(line, ",")[1]);
        let parts: Vec<String> = str::split(line, ",");
        total = total + parts[2].len();
        mk(i);
        total = total + count(mk(i));
        total = total + Vec::len(mk(i));
        i = i + 1;
    }
    let q: String = "QQ" + format!("{}", 8);
    println!("{total}");
    println!("{}", q);
}
"#;

#[test]
fn sb56_an_unbound_vec_result_is_released_at_the_end_of_its_statement() {
    let h = Harness::new();
    h.differential_row("SB-62", SB62_UNBOUND_VEC_RESULTS, "27\nQQ8\n", (95, 95));
    h.assert_legs(18);
}

const SB63_VECS_OF_A_LOOP_BODY_AT_BREAK_AND_CONTINUE: &str = r#"
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 6 {
        i = i + 1;
        let mut v: Vec<String> = Vec::new();
        Vec::push(v, "ab" + format!("{i}"));
        if i % 2 == 0 {
            continue;
        }
        if i == 5 {
            break;
        }
        n = n + v[0].len();
    }
    let q: String = "QQ" + format!("{}", 8);
    println!("{n}");
    println!("{}", q);
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
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 4 {
        i = i + 1;
        let odd: bool = i % 2 == 1;
        let r: T = if odd {
            let a: Vec<i64> = vec![i, 2];
            n = n + a[0];
            x
        } else {
            y
        };
        if !odd {
            continue;
        }
        n = n + 100;
    }
    n
}
fn main() {
    let b: Vec<i64> = vec![9, 9];
    let mut i: i64 = 0;
    let mut n: i64 = 0;
    while i < 4 {
        i = i + 1;
        let odd: bool = i % 2 == 1;
        let sl: &[i64] = if odd {
            let a: Vec<i64> = vec![i, 2];
            n = n + a[0];
            b[..]
        } else {
            b[..]
        };
        if !odd {
            continue;
        }
        n = n + sl[0];
    }
    println!("{}", first(1, 2));
    println!("{}", n);
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
fn clobber(v: &mut Vec<String>) -> i64 {
    (*v)[0] = "zz" + format!("{}", 7);
    let p: String = "QQ" + format!("{}", 8);
    1
}
fn show(x: String, k: i64) -> i64 {
    println!("{}", x);
    k
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, "a" + format!("{}", 1));
    let r: String = v[0] + {
        v[0] = "zz" + format!("{}", 2);
        let p: String = "QQ" + format!("{}", 3);
        p
    };
    println!("{}", r);
    show(v[0], clobber(&mut v));
    println!("{}|{}", v[0], clobber(&mut v));
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
fn put<T>(v: &mut Vec<T>, x: T) { Vec::push(*v, x); }
fn main() {
    let mut v: Vec<String> = Vec::new();
    put(&mut v, "ab" + format!("{}", 1));
    println!("{}", v[0]);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn get<T>(v: &Vec<T>, i: i64) -> Option<T> {
    if i < 0 || i >= Vec::len(*v) { return None; }
    Some((*v)[i])
}
fn lookup(names: &Vec<String>, i: i64) -> String {
    match get(names, i) {
        Some(x) => x,
        None => "?",
    }
}
fn load() -> Option<String> {
    let names: Vec<String> = vec![victim(1), victim(2), victim(3)];
    get(&names, 1)
}
fn main() {
    let mut names: Vec<String> = vec![victim(4), victim(5)];
    println!("{}", lookup(&names, 0));
    let o: Option<String> = load();
    let k = churn();
    match o {
        Some(x) => println!("{}", x),
        None => println!("none"),
    }
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn show(s: String, r: &mut Vec<String>) -> i64 {
    (*r)[0] = victim(9);
    let k = churn();
    println!("{}", s);
    1
}
fn process(r: &mut Vec<String>) -> i64 {
    show((*r)[0], r)
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(1));
    process(&mut v);
    println!("{}", v[0]);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn clobs(r: &mut Vec<String>) -> String {
    (*r)[0] = victim(9);
    let k = churn();
    "z"
}
fn mk(n: i64) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(n));
    v
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(1));
    v[0] = v[0] + clobs(&mut v);
    println!("{}", v[0]);
    v[0] = v[0] + {
        v = mk(5);
        let k = churn();
        "y"
    };
    println!("{}", v[0]);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn clob(v: &mut Vec<String>) -> i64 {
    (*v)[0] = victim(9);
    let k = churn();
    1
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, "x" + format!("yz{}", 1));
    let b: u8 = v[0].as_bytes()[clob(&mut v)];
    let mut s: String = "x" + format!("wz{}", 1);
    let c: u8 = s.as_bytes()[{
        s = victim(2);
        let k = churn();
        1
    }];
    println!("{b}|{c}");
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn tag<T>(x: T) -> i64 {
    let mut names: Vec<String> = Vec::new();
    Vec::push(names, "a" + format!("{}", 1));
    Vec::push(names, "b");
    Vec::len(names)
}
fn main() {
    println!("{}", tag(5));
    println!("{}", tag(true));
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn keep2<T>(a: T, n: i64) -> Option<T> { Some(a) }
fn main() {
    let mut s: String = victim(1);
    let o: Option<String> = keep2(s, { s = victim(2); 1 });
    let k = churn();
    match o {
        Some(x) => println!("{}", x),
        None => println!("none"),
    }
    println!("{}", s);
    println!("{}", k[0]);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn main() {
    let mut v: Vec<String> = Vec::new();
    let mut copies: i64 = 0;
    let mut i: i64 = 0;
    let mut w: Vec<String> = Vec::new();
    while i < 20 {
        Vec::push(v, victim(i));
        if i % 3 == 0 {
            w = v;
        }
        i = i + 1;
    }
    let k = churn();
    println!("{}", v[0] + v[19] + w[0] + w[18]);
    println!("{}", Vec::len(w));
    println!("{}", k[0]);
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
fn got(o: Option<String>) -> String {
    match o {
        Some(x) => { return x; }
        None => { return "none"; }
    }
}
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(1));
    Vec::push(v, victim(2));
    Vec::push(v, victim(3));
    let w: Vec<String> = v;
    let a: String = got(Vec::pop(v));
    Vec::pop(v);
    let x: Vec<String> = v;
    let b: String = got(Vec::pop(v));
    let k = churn();
    println!("{}", a + b);
    println!("{}", w[0] + w[1] + w[2]);
    println!("{}", x[0]);
    println!("{}", Vec::len(v));
    println!("{}", k[0]);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn take_first<T>(v: &mut Vec<T>, w: Vec<T>) -> T {
    let x: T = (*v)[0];
    *v = w;
    x
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(1));
    let mut w: Vec<String> = Vec::new();
    Vec::push(w, victim(2));
    let s: String = take_first(&mut v, w);
    let k = churn();
    println!("{}", s);
    println!("{}", v[0]);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn clobi(r: &mut Vec<String>) -> i64 {
    (*r)[0] = victim(9);
    let k = churn();
    1
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, "x" + format!("yz{}", 1));
    let r: &mut Vec<String> = &mut v;
    let b: u8 = ((*r)[0].as_bytes())[clobi(r)];
    let mut s: String = "x" + format!("yz{}", 1);
    let c: u8 = (s.as_bytes())[{ s = victim(2); let k = churn(); 1 }];
    println!("{b} {c}");
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

struct P { name: String }
fn put<T>(v: &mut Vec<T>, x: T) { Vec::push(*v, x); }
fn main() {
    let mut v: Vec<P> = Vec::new();
    put(&mut v, P { name: victim(1) });
    let mut w: Vec<Option<String>> = Vec::new();
    put(&mut w, Some(victim(2)));
    let k = churn();
    println!("{}", v[0].name);
    match w[0] {
        Some(x) => println!("{}", x),
        None => println!("none"),
    }
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn wrap_local<T>(x: T) -> Option<T> {
    let y: T = x;
    Some(y)
}
fn wrap_pick<T>(c: bool, a: T, b: T) -> Option<T> {
    Some(pick(c, a, b))
}
fn pick<T>(c: bool, a: T, b: T) -> T { if c { return a; } b }
fn main() {
    let o: Option<String> = wrap_local(victim(1));
    let p: Option<String> = wrap_pick(true, victim(2), victim(3));
    let k = churn();
    match o {
        Some(x) => println!("{}", x),
        None => println!("none"),
    }
    match p {
        Some(x) => println!("{}", x),
        None => println!("none"),
    }
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
    for i in 0..s.len() { s[i] = x; }
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, "a" + format!("{}", 1));
    Vec::push(v, "b" + format!("{}", 2));
    let m = Vec::try_as_unique_mut_slice(v);
    let x: String = "c" + format!("{}", 3);
    fill(m, x);
    println!("{}", v[0]);
    println!("{}", v[1]);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn first2<T>(s: &[T]) -> Option<T> {
    if s.len() == 0 { return None; }
    Some(s[0])
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(1));
    let o: Option<String> = first2(Vec::as_slice(v));
    v[0] = victim(2);
    let k = churn();
    match o {
        Some(x) => println!("{}", x),
        None => println!("none"),
    }
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn idx<T: PartialEq>(s: &[T], x: T) -> i64 {
    for i in 0..s.len() {
        if s[i] == x { return i; }
    }
    -1
}
fn first<T>(s: &[T]) -> T { s[0] }
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(1));
    Vec::push(v, victim(2));
    let i: i64 = idx(Vec::as_slice(v), "ab" + format!("{}", 2));
    let o: String = first(Vec::as_slice(v));
    v[0] = victim(3);
    let k = churn();
    println!("{}", format!("{i} ") + o + " " + v[0]);
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
fn two(a: i64, b: i64) -> i64 { a * 10 + b }
fn keep2c(a: String, n: i64) -> String { a }
fn keep2<T>(a: T, n: i64) -> T { a }
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn main() {
    let mut x: i64 = 1;
    let c: i64 = two(x, { x = 2; 3 });
    x = 1;
    let d: i64 = x + { x = 2; 0 };
    x = 1;
    let p: P = P { a: x, b: { x = 2; 3 } };
    x = 1;
    let v: Vec<i64> = vec![x, { x = 2; 3 }];
    x = 1;
    let a: [i64; 2] = [x, { x = 2; 3 }];
    x = 1;
    let f: String = format!("{x}{}", { x = 2; 3 });
    let mut s: String = victim(1);
    let t: String = keep2c(s, { s = victim(2); 1 });
    let u: String = keep2(s, { s = victim(3); 1 });
    println!("{c} {d} {}{} {}{} {}{} {f}", p.a, p.b, v[0], v[1], a[0], a[1]);
    println!("{}", t + u + s);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn keep<T>(x: T) -> T { x }
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 1000 {
        let s: String = keep(victim(i));
        n = n + s.len();
        i = i + 1;
    }
    let k = churn();
    println!("{n}");
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn keep<T>(x: T) -> T { x }
fn pick<T>(c: bool, a: T, b: T) -> T { if c { return a; } b }
fn put<T>(v: &mut Vec<T>, x: T) { Vec::push(*v, x); }
fn main() {
    let mut v: Vec<f64> = Vec::new();
    put(&mut v, keep(1.5));
    let b: bool = pick(true, keep(true), false);
    let n: i64 = pick(false, 1, keep(41));
    println!("{} {b} {n}", v[0]);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn g<T>(v: &mut Vec<T>, x: T) -> T {
    (*v)[0] = x;
    let k = churn();
    (*v)[0]
}
fn same<T: PartialEq>(v: &mut Vec<T>, x: T) -> bool {
    (*v)[0] == g(v, x)
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(1));
    println!("{}", same(&mut v, victim(1)));
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn grab<T>(mut x: T, v: &Vec<T>) -> Option<T> {
    x = (*v)[0];
    Some(x)
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(1));
    let o: Option<String> = grab(victim(5), &v);
    v[0] = victim(2);
    let k = churn();
    match o {
        Some(x) => println!("{}", x),
        None => println!("none"),
    }
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
fn take<T>(v: &mut Vec<T>) -> Option<T> { Vec::pop(*v) }
fn got(o: Option<String>) -> String {
    match o {
        Some(x) => { return x; }
        None => { return "none"; }
    }
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, "a" + format!("{}", 1));
    Vec::push(v, "b" + format!("{}", 2));
    let w: Vec<String> = v;
    let x: String = got(take(&mut v));
    println!("{}", x);
    println!("{}", w[0]);
    println!("{}", v[0]);
}
"#;

#[test]
fn sb81_a_generic_pop_hands_its_element_over() {
    let h = Harness::new();
    h.differential_row("SB-87", SB87_A_GENERIC_POP, "b2\na1\na1\n", (6, 6));
    h.assert_legs(18);
}

const SB88_OPERANDS_REWRITTEN_THROUGH_A_REFERENCE_OR_AS_AN_AGGREGATE: &str = r#"
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

struct P { a: i64, b: i64 }
fn two(a: i64, b: i64) -> i64 { a * 10 + b }
fn bump(r: &mut i64) -> i64 { *r = 2; 3 }
fn bumps(r: &mut String) -> i64 { *r = victim(2); let k = churn(); 1 }
fn keep2c(a: String, n: i64) -> String { a }
fn twoa(a: [i64; 2], n: i64) -> i64 { a[0] * 10 + n }
fn twop(p: P, n: i64) -> i64 { p.a * 10 + n }
fn twov(v: Vec<i64>, n: i64) -> i64 { Vec::len(v) * 10 + n }
fn main() {
    let mut x: i64 = 1;
    let c1: i64 = two(x, bump(&mut x));
    let mut s: String = victim(1);
    let c2: String = keep2c(s, bumps(&mut s));
    let mut a: [i64; 2] = [1, 0];
    let c3: i64 = twoa(a, { a[0] = 2; 3 });
    let mut p: P = P { a: 1, b: 0 };
    let c4: i64 = twop(p, { p.a = 2; 3 });
    let mut v: Vec<i64> = Vec::new();
    Vec::push(v, 1);
    let c5: i64 = twov(v, { Vec::push(v, 2); 3 });
    let k = churn();
    println!("{}", format!("{c1} ") + c2 + format!(" {c3} {c4} {c5}"));
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn keep<T>(x: T) -> T { x }
fn cnt<T>(x: T) -> i64 { 1 }
fn eqk<T: PartialEq>(a: T, b: T) -> bool { keep(a) == keep(b) }
fn pass<T>(a: T) -> i64 { cnt(keep(a)) + cnt(keep(keep(a))) }
fn disc<T>(a: T) -> i64 { keep(a); keep(keep(a)); let n: i64 = cnt(keep(a)); n }
fn wrap<T>(a: T) -> Option<T> { Some(keep(a)) }
fn arr<T>(a: T) -> [T; 2] { [keep(a), keep(a)] }
fn pv<T>(v: &mut Vec<T>, a: T) { Vec::push(*v, keep(a)); (*v)[0] = keep(a); }
fn main() {
    let s: String = victim(1);
    let mut v: Vec<String> = Vec::new();
    pv(&mut v, s);
    let mut w: Vec<i64> = Vec::new();
    pv(&mut w, 3);
    let o: Option<String> = wrap(s);
    let a: [String; 2] = arr(s);
    let b: [i64; 2] = arr(4);
    let k = churn();
    println!("{}", format!("{} {} {} {} {} {} {} {} ", eqk(1, 1), eqk(s, victim(1)), pass(1), pass(s), disc(2.5), disc(s), w[0], b[1]) + v[0] + a[1]);
    match o { Some(x) => println!("{}", x), None => println!("none") }
    match wrap(7) { Some(x) => println!("{x}"), None => println!("none") }
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
fn holdc(mut x: i64, y: i64) -> i64 { let z: i64 = x; x = y; z }
fn holdg<T>(mut x: T, y: T) -> T { let z: T = x; x = y; z }
fn holdl<T>(a: T, y: T) -> T { let mut x: T = a; let z: T = x; x = y; z }
fn holdlc(a: i64, y: i64) -> i64 { let mut x: i64 = a; let z: i64 = x; x = y; z }
fn main() {
    println!("{} {} {} {}", holdc(5, 6), holdg(5, 6), holdl(5, 6), holdlc(5, 6));
    let mut x: i64 = 5;
    let z: i64 = x;
    x = 6;
    println!("{z}");
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn two(a: i64, b: i64) -> i64 { a * 10 + b }
fn keep2<T>(a: T, n: i64) -> T { a }
fn f(mut x: i64, y: i64) -> i64 { two(x, { x = y; 3 }) }
fn fg<T>(mut x: T, y: T) -> T { keep2(x, { x = y; 1 }) }
fn fl(mut x: i64, y: i64) -> i64 { let p: i64 = x + { x = y; 0 }; p }
fn main() {
    println!("{} {} {}", f(1, 2), fg(1, 2), fl(1, 2));
    let s: String = fg(victim(1), victim(2));
    let k = churn();
    println!("{}", s);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn atc(v: &Vec<String>, i: i64) -> &String { &(*v)[i] }
fn at<T>(v: &Vec<T>, i: i64) -> &T { &(*v)[i] }
fn clob(v: &mut Vec<String>) -> i64 { (*v)[0] = victim(8); let k = churn(); 1 }
fn show2(s: String, n: i64) -> String { s + format!("{n}") }
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(1));
    let a: String = show2(*at(&v, 0), clob(&mut v));
    let b: String = show2(*atc(&v, 0), clob(&mut v));
    let k = churn();
    println!("{}", a + b);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn f1(a: i64, b: i64) -> i64 { 100 + b }
fn f2(a: i64, b: i64) -> i64 { 200 + b }
fn main() {
    let mut f: fn(i64, i64) -> i64 = f1;
    println!("{}", f(1, { f = f2; 3 }));
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn lenv(v: Vec<String>, n: i64) -> String { let k = churn(); format!("{}", Vec::len(v)) + v[0] + format!("{n}") }
fn repl(vr: &mut Vec<String>) -> String { lenv(*vr, { *vr = Vec::new(); Vec::push(*vr, victim(3)); 1 }) }
fn grow(vr: &mut Vec<String>) -> String {
    lenv(*vr, { let mut i: i64 = 0; while i < 100 { Vec::push(*vr, victim(i)); i = i + 1; }; 2 })
}
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(1));
    let a: String = repl(&mut v);
    let mut w: Vec<String> = Vec::new();
    Vec::push(w, victim(2));
    let b: String = grow(&mut w);
    let k = churn();
    println!("{}", a + " " + b + format!(" {} {}", Vec::len(v), Vec::len(w)));
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn firstc(a: [i64; 2], n: i64) -> i64 { a[0] }
fn gac(mut a: [i64; 2], x: i64) -> i64 { firstc(a, { a[0] = x; 2 }) }
fn firsts(a: [String; 2], n: i64) -> String { a[0] }
fn gas(mut a: [String; 2], x: String) -> String { firsts(a, { a[0] = x; 2 }) }
fn first<T>(a: [T; 2], n: i64) -> T { a[0] }
fn ga<T>(mut a: [T; 2], x: T) -> T { first(a, { a[0] = x; 2 }) }
fn gl<T>(x: T, y: T) -> T { let mut a: [T; 2] = [x, x]; first(a, { a[0] = y; 2 }) }
fn main() {
    let k = churn();
    println!("{}", format!("{} ", gac([6, 7], 8)) + gas([victim(3), victim(4)], victim(5)) + format!(" {} ", ga([6, 7], 8)) + ga([victim(3), victim(4)], victim(5)) + format!(" {} ", gl(1, 2)) + gl(victim(1), victim(2)));
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
fn setret(mut v: Vec, x: String) -> String { v[0] = x; v[0] }
fn main() {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, "a" + format!("{}", 1));
    println!("{}", setret(v, "b" + format!("{}", 2)));
}
"#;

#[test]
fn sb90_a_vec_parameter_without_an_element_type_asks_for_its_type_argument() {
    let h = Harness::new();
    h.rejects(
        "SB-96",
        SB96_A_VEC_PARAMETER_WITHOUT_AN_ELEMENT_TYPE,
        "E0304",
        "the type `Vec` takes 1 type argument but 0 were given",
    );
    h.assert_legs(1);
}

const SB97_A_STRING_LENT_BY_ADDRESS_IN_A_LOOP: &str = r#"
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn lens(r: &String) -> i64 { (*r).len() }
fn glen<T>(r: &T, n: i64) -> i64 { n }
fn bump(r: &mut String) { *r = *r + "!"; }
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 1000 {
        let s: String = victim(i);
        n = n + lens(&s) + glen(&s, 1);
        let mut t: String = victim(i);
        bump(&mut t);
        n = n + t.len();
        i = i + 1;
    }
    let k = churn();
    println!("{n}");
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
struct P { name: String, age: i64 }
fn half(n: i64) -> Option<i64> { if n % 2 == 0 { return Some(n / 2); } None }
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 300000 {
        let p: P = P { name: "p", age: i };
        let q: P = p;
        n = n + q.age % 7;
        match half(i) {
            Some(h) => { n = n + h % 3; }
            None => { n = n + 1; }
        }
        i = i + 1;
    }
    println!("{n}");
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
struct H { r: &String }
fn main() {
    let s: String = "a" + "b";
    let h: H = H { r: &s };
    println!("{}", *h.r);
}
"#;

const SB99_A_REFERENCE_RETURNED_FROM_A_FUNCTION: &str = r#"
fn dangle() -> &String {
    let s: String = "a" + "b";
    &s
}
fn main() {
    println!("{}", *dangle());
}
"#;

const SB99_A_CLOSURE_THAT_CAPTURES_A_REFERENCE: &str = r#"
fn main() {
    let s: String = "a" + "b";
    let r: &String = &s;
    let f = move |n: i64| -> i64 { n + (*r).len() };
    println!("{}", f(1));
}
"#;

const SB99_AN_EXTERNAL_DECLARATION_AS_A_VALUE: &str = r#"
extern "C" { fn puts(s: &String) -> i64; }
fn main() {
    let f: fn(&String) -> i64 = puts;
    let s: String = "a" + "b";
    println!("{}", f(&s));
}
"#;

const SB99_A_REFERENCE_HANDED_TO_AN_EXTERNAL_CALL: &str = r#"
extern "C" { fn takes(s: &String) -> i64; }
fn main() {
    let s: String = "a" + "b";
    println!("{}", takes(&s));
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

struct P { name: String, city: String, age: i64 }
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 1000 {
        let p: P = P { name: victim(i), city: victim(i + 1), age: i };
        let q: P = p;
        n = n + q.name.len() + p.city.len();
        i = i + 1;
    }
    let k = churn();
    println!("{n}");
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

struct P { name: String, age: i64 }
fn main() {
    let mut p: P = P { name: victim(0), age: 0 };
    let mut i: i64 = 1;
    while i < 1000 {
        p.name = victim(i);
        i = i + 1;
    }
    let k = churn();
    println!("{}", p.name);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn main() {
    let mut a: [String; 3] = [victim(0), victim(1), victim(2)];
    let mut i: i64 = 3;
    while i < 1000 {
        a[i % 3] = victim(i);
        i = i + 1;
    }
    let k = churn();
    println!("{}", a[0] + a[1] + a[2]);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

struct P { name: String, age: i64 }
fn main() {
    let arr: [P; 2] = [P { name: victim(1), age: 1 }, P { name: victim(2), age: 2 }];
    let mut n: i64 = 0;
    for p in arr {
        n = n + p.name.len() + p.age;
    }
    let mut brr: [String; 3] = [victim(3), victim(4), victim(5)];
    let mut acc: String = "";
    for s in brr {
        brr[1] = victim(9);
        let k = churn();
        acc = acc + s;
    }
    println!("{}", format!("{n} ") + acc);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}
struct P { name: String, age: i64 }
struct Q { r: Rc<i64>, name: String }

fn setp<T>(v: &mut Vec<T>, i: i64, x: T) { (*v)[i] = x; }
fn takep<T>(v: &mut Vec<T>) -> Option<T> { Vec::pop(*v) }
fn gotp(o: Option<P>) -> P {
    match o {
        Some(p) => { return p; }
        None => { return P { name: "none", age: 0 }; }
    }
}

fn main() {
    let mut v: Vec<P> = Vec::new();
    Vec::push(v, P { name: victim(1), age: 1 });
    let w: Vec<P> = v;
    v[0] = P { name: victim(2), age: 2 };
    let k = churn();
    setp(&mut v, 0, P { name: victim(3), age: 3 });
    let taken: P = gotp(takep(&mut v));
    Vec::push(v, P { name: victim(4), age: 4 });
    println!("{}", w[0].name + " " + v[0].name + " " + taken.name);
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
use std::str;
struct Person { name: String, city: String }
fn parse(line: String) -> Person {
    let f: Vec<String> = str::split(line, ",");
    Person { name: f[0], city: f[1] }
}
fn main() {
    let mut total: i64 = 0;
    for i in 0..500 {
        let p: Person = parse(format!("alice{i},paris,{i}"));
        total = total + p.name.len() + p.city.len();
    }
    println!("{total}");
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

struct A { s: String }
struct B { a: A, t: String }
fn make(i: i64) -> B { B { a: A { s: victim(i) }, t: victim(i + 1) } }
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 500 {
        let b: B = make(i);
        let c: B = b;
        let a2: A = c.a;
        n = n + a2.s.len() + b.t.len();
        i = i + 1;
    }
    let k = churn();
    println!("{n}");
}
"#;

#[test]
fn sb100_a_struct_inside_a_struct_releases_the_strings_of_both() {
    let h = Harness::new();
    h.differential_row("SB-107", SB107_A_NESTED_STRUCT, "4782\n", (2081, 2081));
    h.assert_legs(18);
}

const SB108_A_GENERIC_INSTANTIATED_WITH_A_STRUCT: &str = r#"
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

struct P { name: String, age: i64 }
fn keep<T>(x: T) -> T { x }
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 500 {
        let p: P = keep(P { name: victim(i), age: i });
        n = n + p.name.len();
        i = i + 1;
    }
    let k = churn();
    println!("{n}");
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

struct S { a: [String; 3], k: i64 }
fn main() {
    let mut v: Vec<[String; 3]> = Vec::new();
    let mut s: S = S { a: [victim(0); 3], k: 0 };
    let mut i: i64 = 1;
    while i < 100 {
        Vec::push(v, [victim(i); 3]);
        s.a = [victim(i + 1); 3];
        i = i + 1;
    }
    let k = churn();
    println!("{}", v[0][0] + s.a[2] + format!("{}", Vec::len(v)));
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

struct P { name: String, age: i64 }
fn gotp(o: Option<P>) -> P {
    match o {
        Some(p) => { return p; }
        None => { return P { name: "none", age: 0 }; }
    }
}
fn main() {
    let mut v: Vec<P> = Vec::new();
    Vec::push(v, P { name: victim(1), age: 1 });
    let w: Vec<P> = v;
    Vec::push(v, P { name: victim(2), age: 2 });
    let taken: P = gotp(Vec::pop(v));
    let mut u: Vec<[String; 2]> = Vec::new();
    Vec::push(u, [victim(3), victim(4)]);
    let t: Vec<[String; 2]> = u;
    Vec::push(u, [victim(5), victim(6)]);
    let k = churn();
    println!("{}", w[0].name + taken.name + t[0][1] + u[1][0] + format!("{} {}", Vec::len(v), Vec::len(t)));
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}
struct Q { r: Rc<i64>, name: String }

fn main() {
    let q: Q = Q { r: Rc::new(5), name: victim(1) };
    let q2: Q = q;
    let k = churn();
    println!("{}", q2.name + q.name + format!("{}", Rc::get(q2.r) + Rc::get(q.r)));
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

struct P { name: String, age: i64 }
fn clob(p: &mut P) -> i64 {
    (*p).name = victim(1);
    let k = churn();
    1
}
fn show(s: String, n: i64) -> String { s }
fn main() {
    let mut p: P = P { name: victim(1), age: 0 };
    let s: String = show(p.name, clob(&mut p));
    println!("{}", s + p.name);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

struct S { a: [String; 3], k: i64 }
fn take(a: [String; 3]) -> i64 { a[0].len() }
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 200 {
        n = n + take([victim(i); 3]);
        let s: S = S { a: [victim(i + 1); 3], k: i };
        n = n + s.a[1].len() + s.k % 3;
        i = i + 1;
    }
    let k = churn();
    println!("{n}");
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
enum Shape { Named(String, i64), Sized(i64), Blank }
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn main() {
    let mut v: Vec<Option<i64>> = Vec::new();
    Vec::push(v, Some(11));
    Vec::push(v, Some(22));
    Vec::push(v, Some(33));
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 3 {
        match v[i] {
            Some(x) => { n = n * 100 + x; }
            None => { n = n * 100; }
        }
        i = i + 1;
    }
    let mut w: Vec<Shape> = Vec::new();
    Vec::push(w, Shape::Sized(7));
    Vec::push(w, Shape::Named(victim(1), 5));
    Vec::push(w, Shape::Blank);
    let mut j: i64 = 0;
    let mut acc: String = "";
    while j < 3 {
        match w[j] {
            Shape::Named(t, k) => { acc = acc + t + format!("{k}"); }
            Shape::Sized(k) => { acc = acc + format!("s{k}"); }
            Shape::Blank => { acc = acc + "b"; }
        }
        j = j + 1;
    }
    println!("{}", format!("{n} ") + acc);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn hold(o: Option<String>) -> i64 {
    match o {
        Some(x) => { return x.len(); }
        None => { return 0; }
    }
}
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 20 {
        let o: Option<String> = Some(victim(i));
        n = n + hold(o);
        let p: Option<String> = if i % 2 == 0 { Some(victim(i + 1)) } else { None };
        match p {
            Some(y) => { n = n + y.len(); }
            None => { n = n + 1; }
        }
        i = i + 1;
    }
    println!("{n}");
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
enum Shape { Named(String, i64), Tagged(String), Blank }
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn make(i: i64) -> Shape {
    if i % 3 == 0 { return Shape::Named(victim(i), i); }
    if i % 3 == 1 { return Shape::Tagged(victim(i + 1)); }
    Shape::Blank
}
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 12 {
        let s: Shape = make(i);
        match s {
            Shape::Named(t, k) => { n = n + t.len() + k; }
            Shape::Tagged(t) => { n = n + t.len(); }
            Shape::Blank => { n = n + 1; }
        }
        i = i + 1;
    }
    println!("{n}");
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn main() {
    let mut n: i64 = 0;
    let mut v: Vec<Option<String>> = Vec::new();
    Vec::push(v, Some(victim(1)));
    Vec::push(v, None);
    Vec::push(v, Some(victim(2)));
    let w: Vec<Option<String>> = v;
    Vec::push(v, Some(victim(3)));
    match Vec::pop(v) {
        Some(t) => {
            match t {
                Some(x) => { n = n + x.len(); }
                None => { n = n + 1; }
            }
        }
        None => { n = n + 100; }
    }
    let mut i: i64 = 0;
    while i < 3 {
        match w[i] {
            Some(x) => { n = n + x.len(); }
            None => { n = n + 7; }
        }
        i = i + 1;
    }
    println!("{n}");
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
struct P { a: String, b: i64 }
enum Box3 { Pair(P), Trio([String; 3]), Empty }
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 8 {
        let b: Box3 = if i % 2 == 0 {
            Box3::Pair(P { a: victim(i), b: i })
        } else {
            Box3::Trio([victim(i), victim(i + 1), victim(i + 2)])
        };
        match b {
            Box3::Pair(p) => { n = n + p.a.len() + p.b; }
            Box3::Trio(t) => { n = n + t[0].len() + t[2].len(); }
            Box3::Empty => { n = n + 1; }
        }
        i = i + 1;
    }
    println!("{n}");
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
enum Tag2 { Some3(String), None3 }
struct Holder { t: Tag2, name: String }
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn wrap<T>(x: T) -> Option<T> { Some(x) }
fn unwrap_or<T>(o: Option<T>, d: T) -> T {
    match o {
        Some(x) => { return x; }
        None => { return d; }
    }
}
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 10 {
        let o: Option<String> = wrap(victim(i));
        let s: String = unwrap_or(o, "zz");
        n = n + s.len();
        let h: Holder = Holder { t: Tag2::Some3(victim(i + 1)), name: victim(i + 2) };
        let g: Holder = h;
        match g.t {
            Tag2::Some3(x) => { n = n + x.len(); }
            Tag2::None3 => { n = n + 1; }
        }
        n = n + g.name.len();
        i = i + 1;
    }
    println!("{n}");
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn get(i: i64) -> Option<String> { if i % 3 == 0 { return None; }  Some(victim(i)) }
struct P { a: String, b: i64 }
fn mk(i: i64) -> P { P { a: victim(i), b: i } }
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 9 {
        match get(i) {
            Some(v) => { n = n + v.len(); }
            None => { n = n + 1; }
        }
        n = n + mk(i).a.len();
        match Some(victim(i + 1)) {
            Some(w) => { n = n + w.len(); }
            None => { n = n + 1; }
        }
        i = i + 1;
    }
    println!("{n}");
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn get(i: i64) -> Result<String, i64> {
    if i % 4 == 3 { return Err(i); }
    Ok(victim(i))
}
fn chain(i: i64) -> Result<String, i64> {
    let s: String = get(i)?;
    Ok(s + "!")
}
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 8 {
        match chain(i) {
            Ok(s) => { n = n + s.len(); }
            Err(e) => { n = n + e % 5; }
        }
        i = i + 1;
    }
    println!("{n}");
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
struct P { a: String, b: i64 }
static mut g: String = "g0";
static mut h: P = P { a: "h0", b: 0 };
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 6 {
        g = victim(i);
        h = P { a: victim(i + 1), b: i };
        n = n + g.len() + h.a.len();
        i = i + 1;
    }
    println!("{n}");
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
static mut g: String = "init";
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 40 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}
fn setg(n: i64) -> i64 {
    g = victim(n);
    let k = churn();
    1
}
fn keeps(a: String, n: i64) -> String { a }
fn keeps_gen<T>(a: T, n: i64) -> T { a }
fn main() {
    g = victim(1);
    let t: String = keeps(g, setg(2));
    let k1 = churn();
    let u: String = keeps_gen(g, setg(3));
    let k2 = churn();
    println!("{}", t + u);
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn pick<T>(a: T, b: T) -> T {
    let mut i: i64 = 0;
    let mut r: T = b;
    while i < 6 {
        i = i + 1;
        r = if i % 2 == 0 {
            let k: String = "x" + format!("{i}");
            a
        } else { b };
        if i == 3 { continue; }
        if i == 5 { break; }
    }
    r
}
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 4 {
        let a: String = victim(i);
        let b: String = victim(i + 1);
        let r: String = pick(a, b);
        n = n + r.len();
        i = i + 1;
    }
    println!("{n}");
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
fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn churn() -> Vec<String> {
    let mut keep: Vec<String> = Vec::new();
    let mut i: i64 = 10;
    while i < 50 {
        Vec::push(keep, "Q" + format!("{i}"));
        i = i + 1;
    }
    keep
}

fn mk(n: i64) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    Vec::push(v, victim(n));
    v
}
fn main() {
    let v: Vec<String> = mk(1);
    let g = move || -> Vec<String> { v };
    let w: Vec<String> = g();
    let x: Vec<String> = g();
    println!("{}", Vec::len(g()));
    let k = churn();
    println!("{}", w[0] + x[0]);
    println!("{}", v[0]);
}
"#;

#[test]
fn sb118_a_closure_that_returns_its_captured_vec_hands_out_a_share() {
    let h = Harness::new();
    h.differential_row(
        "SB-125",
        SB125_A_CLOSURE_HANDS_OUT_A_SHARE_OF_ITS_CAPTURE,
        "1\nab1ab1\nab1\n",
        (86, 86),
    );
    h.assert_legs(18);
}

const SB126_A_SORT_THROUGH_A_MUTABLE_SLICE_KEEPS_ONE_SHARE: &str = r#"
use std::sort;

fn victim(n: i64) -> String { "ab" + format!("{n}") }
fn main() {
    let mut n: i64 = 0;
    let mut i: i64 = 0;
    while i < 4 {
        let mut v: Vec<String> = Vec::new();
        Vec::push(v, victim(i + 3));
        Vec::push(v, victim(i + 1));
        Vec::push(v, victim(i + 2));
        let s = Vec::try_as_unique_mut_slice(v);
        sort::strings(s);
        n = n + s[0].len();
        i = i + 1;
    }
    println!("{n}");
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

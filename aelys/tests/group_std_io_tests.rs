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

const MODULES: &[&str] = &["io.aelys", "result.aelys", "str.aelys", "vec.aelys"];

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

// `exit` or `abort` never reaches it and can carry no allocation evidence at all
fn strip_stats(stderr: &str) -> String {
    stderr
        .lines()
        .filter(|l| !l.contains("[rc] allocs="))
        .map(|l| format!("{l}\n"))
        .collect()
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

struct Want {
    stdout: &'static str,
    stderr: &'static str,
    code: i32,
    env: &'static [(&'static str, &'static str)],
    stats: Option<(i64, i64)>,
}

impl Want {
    fn out(stdout: &'static str) -> Want {
        Want { stdout, stderr: "", code: 0, env: &[], stats: Some((0, 0)) }
    }
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

    // `needs std.io` resolves under the root file, so the library is copied beside every root
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

    fn row(&self, id: &str, src: &str, want: &Want) {
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
                for (k, v) in want.env {
                    cmd.env(k, v);
                }
                let out = cmd.output().expect("run compiled exe");
                let seen_out = String::from_utf8_lossy(&out.stdout).into_owned();
                let seen_err = String::from_utf8_lossy(&out.stderr).into_owned();
                self.legs.set(self.legs.get() + 1);
                assert_eq!(
                    exit_code(&out.status),
                    want.code,
                    "{id} at {tag}/{alloc_name} must exit {}\nstderr:\n{seen_err}",
                    want.code
                );
                assert_eq!(
                    seen_out, want.stdout,
                    "{id} at {tag}/{alloc_name}: stdout MUST be {:?}",
                    want.stdout
                );
                assert_eq!(
                    strip_stats(&seen_err),
                    want.stderr,
                    "{id} at {tag}/{alloc_name}: stderr MUST be {:?}",
                    want.stderr
                );
                if let Some(expected) = want.stats {
                    let got = parse_stats(&seen_err).unwrap_or_else(|| {
                        panic!("{id} at {tag}/{alloc_name}: no [rc] stats line\nstderr:\n{seen_err}")
                    });
                    assert_eq!(
                        got, expected,
                        "{id} at {tag}/{alloc_name}: MUST be allocs={} frees={}",
                        expected.0, expected.1
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

    fn rejects(&self, id: &str, src: &str, code: &str) {
        let root = self.stage(id, "reject", src);
        let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
            Ok(_) => panic!("{id}: MUST be rejected\n{src}"),
            Err(rendered) => rendered,
        };
        assert!(
            rendered.contains(code),
            "{id}: the rejection MUST be {code}\n{src}\nrendered:\n{rendered}"
        );
    }
}

const IO_WRITES: &str = r#"
needs std.io

fn main() -> i64 {
    let hi: [u8;6] = [104, 101, 108, 108, 111, 10]
    let bad: [u8;4] = [98, 97, 100, 10]
    let mut none: [u8;0] = []
    println(io.write_out(hi[..]))
    println(io.write_err(bad[..]))
    println(io.write_all(1, none[..]))
    println(io.write_all(2, none[..]))
    return 0
}
"#;

#[test]
fn group_std_io_writes_a_line_to_stdout_and_to_stderr_without_allocating() {
    let h = Harness::new();
    h.row(
        "STD-IO-1",
        IO_WRITES,
        &Want {
            stdout: "hello\n6\n4\n0\n0\n",
            stderr: "bad\n",
            code: 0,
            env: &[],
            stats: Some((0, 0)),
        },
    );
    h.assert_legs(6);
}

const IO_LONG: &str = r#"
needs std.io

fn main() -> i64 {
    let mut b: [u8;40] = [
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
        0, 0, 0, 0, 0, 0, 0, 0, 0, 0
    ]
    for i in 0..39 { b[i] = 120 }
    b[39] = 10
    println(io.write_out(b[..]))
    return 0
}
"#;

#[test]
fn group_std_io_write_all_reports_every_byte_of_a_long_buffer() {
    let h = Harness::new();
    h.row(
        "STD-IO-2",
        IO_LONG,
        &Want::out("xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n40\n"),
    );
    h.assert_legs(6);
}

const IO_ENV: &str = r#"
needs std.io

fn main() -> i64 {
    let set: [u8;13] = [71, 82, 79, 85, 80, 95, 83, 84, 68, 95, 73, 79, 0]
    let missing: [u8;15] = [71, 82, 79, 85, 80, 95, 83, 84, 68, 95, 78, 79, 80, 69, 0]
    let unterminated: [u8;12] = [71, 82, 79, 85, 80, 95, 83, 84, 68, 95, 73, 79]
    let mut empty: [u8;0] = []
    println(io.env_len(set[..]))
    println(io.env_len(missing[..]))
    println(io.env_len(unterminated[..]))
    println(io.env_len(empty[..]))
    if io.env_is_set(set[..]) { println(1) } else { println(0) }
    if io.env_is_set(missing[..]) { println(1) } else { println(0) }
    return 0
}
"#;

#[test]
fn group_std_io_env_len_answers_for_a_set_a_missing_and_an_unterminated_name() {
    let h = Harness::new();
    h.row(
        "STD-IO-3",
        IO_ENV,
        &Want {
            stdout: "7\n-1\n-1\n-1\n1\n0\n",
            stderr: "",
            code: 0,
            env: &[("GROUP_STD_IO", "abcdefg")],
            stats: Some((0, 0)),
        },
    );
    h.assert_legs(6);
}

const IO_TIME: &str = r#"
needs std.io

fn main() -> i64 {
    let a: i64 = io.unix_time()
    let b: i64 = io.unix_time()
    if a > 1700000000 { println(1) } else { println(0) }
    if b >= a { println(1) } else { println(0) }
    if a < 4000000000 { println(1) } else { println(0) }
    return 0
}
"#;

#[test]
fn group_std_io_unix_time_is_a_plausible_epoch_second_and_never_goes_backwards() {
    let h = Harness::new();
    h.row("STD-IO-4", IO_TIME, &Want::out("1\n1\n1\n"));
    h.assert_legs(6);
}

const IO_EXIT: &str = r#"
needs std.io

fn main() -> i64 {
    let m: [u8;3] = [111, 107, 10]
    println(io.write_out(m[..]))
    io.exit_with(42)
    println(999)
    return 0
}
"#;

#[test]
fn group_std_io_exit_with_carries_the_code_and_the_statement_after_it_never_runs() {
    let h = Harness::new();
    h.row(
        "STD-IO-5",
        IO_EXIT,
        &Want {
            stdout: "ok\n3\n",
            stderr: "",
            code: 42,
            env: &[],
            stats: None,
        },
    );
    h.assert_legs(6);
}

const IO_ABORT: &str = r#"
needs std.io

fn main() -> i64 {
    let m: [u8;3] = [111, 107, 10]
    println(io.write_out(m[..]))
    io.abort_now()
    println(999)
    return 0
}
"#;

#[test]
fn group_std_io_abort_now_leaves_through_sigabrt_and_never_reaches_the_next_statement() {
    let h = Harness::new();
    h.row(
        "STD-IO-6",
        IO_ABORT,
        &Want {
            stdout: "ok\n3\n",
            stderr: "",
            code: 134,
            env: &[],
            stats: None,
        },
    );
    h.assert_legs(6);
}

const IO_BESIDE_RUNTIME: &str = r#"
needs std.io
needs std.str

fn main() -> i64 {
    let n: [u8;15] = [65, 69, 76, 89, 83, 95, 82, 67, 95, 83, 84, 65, 84, 83, 0]
    let m: [u8;3] = [111, 107, 10]
    if io.env_is_set(n[..]) { println(1) } else { println(0) }
    println(io.write_out(m[..]))
    println(str.repeat("ab", 3))
    return 0
}
"#;

#[test]
fn group_std_io_externs_link_beside_the_runtime_symbols_they_share() {
    let h = Harness::new();
    h.row(
        "STD-IO-7",
        IO_BESIDE_RUNTIME,
        &Want {
            stdout: "1\nok\n3\nababab\n",
            stderr: "",
            code: 0,
            env: &[],
            stats: Some((3, 0)),
        },
    );
    h.assert_legs(6);
}

// the exact spelling std/io.aelys uses, called the way std/io.aelys must never call it
const IO_EXTERN_UNGUARDED: &str = r#"
needs std.io

unsafe extern nogc fn time(slot: u64) -> i64

fn main() -> i64 {
    return time(0)
}
"#;

#[test]
fn group_std_io_calling_the_modules_extern_outside_unsafe_is_still_e0617() {
    let h = Harness::new();
    h.rejects("STD-IO-8", IO_EXTERN_UNGUARDED, "E0617");
}

const IO_EXTERN_RETURNS_REF: &str = r#"
unsafe extern nogc fn getenv(name: &u8) -> &u8

fn main() -> i64 {
    return 0
}
"#;

#[test]
fn group_std_io_an_extern_returning_a_reference_is_still_e0615() {
    let h = Harness::new();
    h.rejects("STD-IO-9", IO_EXTERN_RETURNS_REF, "E0615");
}

const IO_EXTERN_COLLIDES_WITH_WRAPPER: &str = r#"
unsafe extern nogc fn exit(code: i32)

pub nogc fn exit(code: i32) {
    unsafe { exit(code) }
}

fn main() -> i64 {
    return 0
}
"#;

#[test]
fn group_std_io_a_wrapper_colliding_with_its_extern_is_rejected_under_a_meaningless_headline() {
    let h = Harness::new();
    let root = h.stage("STD-IO-10", "reject", IO_EXTERN_COLLIDES_WITH_WRAPPER);
    let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
        Ok(_) => panic!("STD-IO-10: MUST be rejected\n{IO_EXTERN_COLLIDES_WITH_WRAPPER}"),
        Err(rendered) => rendered.to_string(),
    };
    assert_eq!(
        rendered.lines().next(),
        Some("error[E0301]: expected `dynamic`, found `dynamic`"),
        "STD-IO-10: PIN, DO NOT REPAIR: the headline must still name no symbol and no \
         reason\nrendered:\n{rendered}"
    );
    assert!(
        rendered.contains("= note: duplicate function definition 'exit'"),
        "STD-IO-10: the note is the only line that says what is wrong\nrendered:\n{rendered}"
    );
}

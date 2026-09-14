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

    // `needs std.result` resolves under the root file, so the library is copied beside every root
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
                    "{id} at {tag}/{alloc_name}: stdout MUST be {stdout:?}\nstderr:\n{seen_err}"
                );
            }
        }
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

    // ld names the dead call at -o0 only, so this row never compares its words across levels
    fn link_refusal_at_every_level(&self, id: &str, src: &str, wants: &[&str]) {
        for (tag, opt) in LEVELS {
            let root = self.stage(id, tag, src);
            let rendered =
                match compile_file_with_llvm_variant(&root, *opt, false, RuntimeVariant::Rc) {
                    Ok(()) => panic!("{id} at {tag}: the link cannot resolve the symbol"),
                    Err(err) => err.to_string(),
                };
            if linker_unavailable(&rendered) {
                self.linker_skips.set(self.linker_skips.get() + 1);
                assert!(
                    linker_skip_declared(),
                    "{id}: no linker, and AELYS_ALLOW_LINKER_SKIP is not set"
                );
                return;
            }
            self.legs.set(self.legs.get() + 1);
            for want in wants {
                assert!(
                    rendered.contains(want),
                    "{id} at {tag}: the refusal MUST say {want:?}\nrendered:\n{rendered}"
                );
            }
            assert!(
                !rendered.contains("[E0901]") && !rendered.contains("compiler bug"),
                "{id} at {tag}: a missing library is not a compiler bug\nrendered:\n{rendered}"
            );
            assert!(
                !exe_path_for(&root).exists(),
                "{id} at {tag}: a refused link must leave no executable"
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

const ESCAPE_SAYS: &str = "does not live long enough; it is borrowed and the borrow is used after";

const S4C_MATCH_BINDER_I64: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let anchor: i64 = 0
    let mut r: &i64 = &anchor
    {
        let o: Option<i64> = Option::Some(7)
        match o {
            Option::Some(t) => r = &t,
            Option::None => r = &anchor,
        }
    }
    println(*r)
    return 0
}
"#;

const S4C_MATCH_BINDER_BYTES: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let anchor: string = "zz"
    let mut b: &[u8] = anchor.bytes
    {
        let o: Option<string> = Option::Some("hi" + "!")
        match o {
            Option::Some(t) => b = t.bytes,
            Option::None => b = anchor.bytes,
        }
    }
    println(b[0] as i64)
    return 0
}
"#;

const S4C_MATCH_BINDER_USER_ENUM: &str = r#"
enum B { F(i64), G }

fn main() -> i64 {
    let anchor: i64 = 0
    let mut r: &i64 = &anchor
    {
        let o: B = B::F(7)
        match o {
            B::F(t) => r = &t,
            B::G => r = &anchor,
        }
    }
    println(*r)
    return 0
}
"#;

// the arm binder gets one slot per loop, so the next iteration overwrites what the borrow names
const S4C_MATCH_BINDER_IN_A_LOOP: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let anchor: i64 = 0
    let mut r: &i64 = &anchor
    for i in 0..2 {
        let o: Option<i64> = Option::Some(i * 10 + 1)
        match o {
            Option::Some(t) => { if i == 0 { r = &t } }
            Option::None => r = &anchor
        }
    }
    println(*r)
    return 0
}
"#;

const S4C_CATCH_BINDER: &str = r#"
enum Result<T, E> { Ok(T), Err(E) }

fn get() -> Result<i64, i64> { return Result::Err(7) }

fn main() -> i64 {
    let anchor: i64 = 0
    let mut r: &i64 = &anchor
    {
        let n: i64 = get() catch |e| {
            r = &e
            0
        }
        println(n)
    }
    println(*r)
    return 0
}
"#;

const S4C_LET_TWIN: &str = r#"
fn main() -> i64 {
    let anchor: i64 = 0
    let mut r: &i64 = &anchor
    {
        let t: i64 = 7
        r = &t
    }
    println(*r)
    return 0
}
"#;

#[test]
fn s4c_1_a_loan_rooted_in_a_match_binder_dies_with_its_arm() {
    let h = Harness::new();
    for (id, src) in [
        ("S4c-1a", S4C_MATCH_BINDER_I64),
        ("S4c-1b", S4C_MATCH_BINDER_BYTES),
        ("S4c-1c", S4C_MATCH_BINDER_USER_ENUM),
        ("S4c-1d", S4C_MATCH_BINDER_IN_A_LOOP),
    ] {
        h.rejects_at_every_level(id, src, "E0722", ESCAPE_SAYS);
    }
    h.assert_legs(16);
}

#[test]
fn s4c_2_a_catch_binder_is_the_same_door() {
    let h = Harness::new();
    h.rejects_at_every_level("S4c-2", S4C_CATCH_BINDER, "E0722", ESCAPE_SAYS);
    h.assert_legs(4);
}

#[test]
fn s4c_3_the_let_twin_is_the_control_the_rule_already_had() {
    let h = Harness::new();
    h.rejects_at_every_level("S4c-3", S4C_LET_TWIN, "E0722", ESCAPE_SAYS);
    h.assert_legs(4);
}

const S4C_BINDER_READ: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let o: Option<i64> = Option::Some(7)
    match o {
        Option::Some(t) => println(t),
        Option::None => println(0),
    }
    return 0
}
"#;

const S4C_BINDER_BORROWED_INSIDE: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let o: Option<i64> = Option::Some(7)
    match o {
        Option::Some(t) => {
            let p: &i64 = &t
            println(*p)
        }
        Option::None => println(0)
    }
    return 0
}
"#;

const S4C_BINDER_BYTES_INSIDE: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let o: Option<string> = Option::Some("hi")
    match o {
        Option::Some(s) => {
            let b: &[u8] = s.bytes
            println(b[0] as i64)
            println(s)
        }
        Option::None => println(0)
    }
    return 0
}
"#;

const S4C_BINDER_PASSED_BY_REFERENCE: &str = r#"
needs Option from std.result

fn take(p: &i64) -> i64 { return *p }

fn main() -> i64 {
    let o: Option<i64> = Option::Some(7)
    match o {
        Option::Some(t) => println(take(&t)),
        Option::None => println(0)
    }
    return 0
}
"#;

const S4C_BINDER_NESTED: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let o: Option<i64> = Option::Some(7)
    match o {
        Option::Some(t) => {
            let i: Option<i64> = Option::Some(t + 1)
            match i {
                Option::Some(t) => println(t),
                Option::None => println(0)
            }
        }
        Option::None => println(0)
    }
    return 0
}
"#;

const S4C_BINDER_IN_A_LOOP: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let mut n: i64 = 0
    for i in 0..3 {
        let o: Option<i64> = Option::Some(i)
        match o {
            Option::Some(t) => n = n + t,
            Option::None => n = n + 0
        }
    }
    println(n)
    return 0
}
"#;

const S4C_CATCH_BINDER_INSIDE: &str = r#"
enum Result<T, E> { Ok(T), Err(E) }

fn get() -> Result<i64, i64> { return Result::Err(7) }

fn main() -> i64 {
    let n: i64 = get() catch |e| e + 1
    println(n)
    return 0
}
"#;

#[test]
fn s4c_4_a_binder_that_stays_inside_its_arm_still_runs() {
    let h = Harness::new();
    for (id, src, stdout) in [
        ("S4c-4a", S4C_BINDER_READ, "7\n"),
        ("S4c-4b", S4C_BINDER_BORROWED_INSIDE, "7\n"),
        ("S4c-4c", S4C_BINDER_BYTES_INSIDE, "104\nhi\n"),
        ("S4c-4d", S4C_BINDER_PASSED_BY_REFERENCE, "7\n"),
        ("S4c-4e", S4C_BINDER_NESTED, "8\n"),
        ("S4c-4f", S4C_BINDER_IN_A_LOOP, "3\n"),
        ("S4c-4g", S4C_CATCH_BINDER_INSIDE, "8\n"),
    ] {
        h.value_row(id, src, stdout);
    }
    h.assert_legs(56);
}

fn binary_mismatch(body: &str) -> String {
    format!("fn main() -> i64 {{\n{body}\n    println(0)\n    return 0\n}}\n")
}

#[test]
fn s4c_5_a_binary_operand_mismatch_names_the_left_operand_first() {
    let h = Harness::new();
    for (id, body, says) in [
        (
            "S4c-5a",
            "    let x: i64 = 1 + true",
            "expected `i64`, found `bool`",
        ),
        (
            "S4c-5b",
            "    let x: i64 = 1 - true",
            "expected `i64`, found `bool`",
        ),
        (
            "S4c-5c",
            "    let x: i64 = 6 / true",
            "expected `i64`, found `bool`",
        ),
        (
            "S4c-5d",
            "    let x: i64 = 1 % true",
            "expected `i64`, found `bool`",
        ),
        (
            "S4c-5e",
            "    let x: i64 = 1 << true",
            "expected `i64`, found `bool`",
        ),
        (
            "S4c-5f",
            "    let b: bool = 1 < \"a\"",
            "expected `i64`, found `string`",
        ),
        (
            "S4c-5g",
            "    let x: i64 = 1 + \"a\"",
            "expected `i64`, found `string`",
        ),
        (
            "S4c-5h",
            "    let s: string = \"a\" + 1",
            "expected `string`, found `i64`",
        ),
        (
            "S4c-5i",
            "    let c: char = 'b'\n    let t: string = \"a\" + c",
            "expected `string`, found `char`",
        ),
        (
            "S4c-5j",
            "    let c: char = 'b'\n    let b: bool = c < \"a\"",
            "expected `char`, found `string`",
        ),
        (
            "S4c-5k",
            "    let c: char = 'b'\n    let b: bool = \"a\" < c",
            "expected `string`, found `char`",
        ),
    ] {
        h.rejects_at_every_level(id, &binary_mismatch(body), "E0301", says);
    }
    h.assert_legs(44);
}

const S4C_ARITHMETIC_STILL_RUNS: &str = r#"
fn main() -> i64 {
    let a: i64 = 7 + 3
    let b: i64 = a - 2
    let c: i64 = b * 3
    let d: i64 = c / 4
    let e: i64 = d % 5
    let f: i64 = e << 2
    let g: i64 = f | 1
    println(g)
    if a < c { println(1) } else { println(0) }
    if a == 10 { println(1) } else { println(0) }
    println("ab" + "cd")
    if 'a' < 'b' { println(1) } else { println(0) }
    return 0
}
"#;

#[test]
fn s4c_6_the_operators_that_type_check_still_answer() {
    let h = Harness::new();
    h.value_row("S4c-6", S4C_ARITHMETIC_STILL_RUNS, "5\n1\n1\nabcd\n1\n");
    h.assert_legs(8);
}

const S4C_MISSING_EXTERN: &str = r#"
unsafe extern fn s4c_no_such_symbol_anywhere() -> i64

fn main() -> i64 {
    if false { unsafe { println(s4c_no_such_symbol_anywhere()) } }
    println(1)
    return 0
}
"#;

const S4C_NO_EXTERN: &str = r#"
fn main() -> i64 {
    println(1)
    return 0
}
"#;

#[test]
fn s4c_7_a_missing_external_symbol_is_the_toolchains_refusal() {
    let h = Harness::new();
    h.link_refusal_at_every_level(
        "S4c-7",
        S4C_MISSING_EXTERN,
        &[
            "[E0903]",
            "the toolchain refused",
            "s4c_no_such_symbol_anywhere",
            "add `-L <dir> -l <name>`",
        ],
    );
    h.assert_legs(4);
}

#[test]
fn s4c_8_the_same_program_without_the_declaration_links_and_runs() {
    let h = Harness::new();
    h.value_row("S4c-8", S4C_NO_EXTERN, "1\n");
    h.assert_legs(8);
}

#[test]
fn s4c_9_the_link_fault_is_registered_and_reads_as_the_toolchains() {
    let info = aelys_common::diagnostic::registry::lookup("E0903")
        .expect("E0903 must be a registered code");
    assert_eq!(info.title, "the toolchain refused");
    assert!(
        info.explanation.contains("A missing library"),
        "E0903 must name a missing library as one of its causes:\n{}",
        info.explanation
    );
    let compiler = aelys_common::diagnostic::registry::lookup("E0901")
        .expect("E0901 must be a registered code");
    assert!(
        compiler
            .explanation
            .contains("Your program is not at fault"),
        "E0901 is the code a link failure must not reach:\n{}",
        compiler.explanation
    );
}

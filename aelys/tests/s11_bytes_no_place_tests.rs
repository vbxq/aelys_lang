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

    // the artifact is run at every level under both allocators: a compile that only type-checks
    fn value_row(&self, id: &str, src: &str, stdout: &str) {
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

const NO_PLACE_SAYS: &str = "[no-place] the receiver of `.bytes` denotes no storage, so it has no \
                             address. bind it to a name first and use that binding";

const S11_VIEW_OF_A_TEMPORARY: &str = r#"
fn mkheap(n: i64) -> string {
    return string::substring_bytes("ABCDEFGH", 0, n)
}

fn churn(n: i64) -> i64 {
    let mut acc: i64 = 0
    let mut i: i64 = 0
    while i < n {
        let t: string = string::substring_bytes("0123456789abcdef", 0, (i % 9) + 2)
        acc = acc + t.len
        i = i + 1
    }
    return acc
}

fn main() -> i64 {
    let s: &[u8] = mkheap(4).bytes
    println(churn(200))
    println(s[0] as i64)
    return 0
}
"#;

const S11_VIEW_OF_A_TEMPORARY_IN_AN_ARGUMENT: &str = r#"
nogc fn first(b: &[u8]) -> i64 { return b[0] as i64 }

fn mkheap() -> string {
    return string::substring_bytes("ABCDEFGH", 0, 4)
}

fn main() -> i64 {
    println(first(mkheap().bytes))
    return 0
}
"#;

const S11_VIEW_OF_AN_INTERPOLATION: &str = r#"
fn main() -> i64 {
    let n: i64 = 7
    let b: &[u8] = "v{n}".bytes
    println(b.len)
    return 0
}
"#;

#[test]
fn s11_1_a_byte_view_of_a_receiver_with_no_storage_is_refused_at_every_level() {
    let h = Harness::new();
    h.rejects_at_every_level("S1.1-1a", S11_VIEW_OF_A_TEMPORARY, "E0421", NO_PLACE_SAYS);
    h.rejects_at_every_level(
        "S1.1-1b",
        S11_VIEW_OF_A_TEMPORARY_IN_AN_ARGUMENT,
        "E0421",
        NO_PLACE_SAYS,
    );
    h.rejects_at_every_level(
        "S1.1-1c",
        S11_VIEW_OF_AN_INTERPOLATION,
        "E0421",
        NO_PLACE_SAYS,
    );
    h.assert_legs(12);
}

const S11_LITERAL_RECEIVER: &str = r#"
fn churn(n: i64) -> i64 {
    let mut acc: i64 = 0
    let mut i: i64 = 0
    while i < n {
        let t: string = string::substring_bytes("0123456789abcdef", 0, (i % 9) + 2)
        acc = acc + t.len
        i = i + 1
    }
    return acc
}

fn main() -> i64 {
    let lb: &[u8] = "hi".bytes
    println(churn(300))
    println(lb[0] as i64)
    println(lb[1] as i64)
    println(lb.len)
    println(("hi").bytes[0] as i64)
    println("hi".bytes.len)
    return 0
}
"#;

#[test]
fn s11_2_a_literal_receiver_is_static_storage_so_it_still_compiles_and_runs() {
    let h = Harness::new();
    h.value_row(
        "S1.1-2",
        S11_LITERAL_RECEIVER,
        "1791\n104\n105\n2\n104\n2\n",
    );
    h.assert_legs(8);
}

const S11_STD_BYTES_SHAPES: &str = r#"
needs std.str
needs std.io

struct Box { s: string }

fn churn(n: i64) -> i64 {
    let mut acc: i64 = 0
    let mut i: i64 = 0
    while i < n {
        let t: string = string::substring_bytes("0123456789abcdef", 0, (i % 9) + 2)
        acc = acc + t.len
        i = i + 1
    }
    return acc
}

fn main() -> i64 {
    let s: string = "éléphant sur la route"
    println(str.char_count(s))
    println(str.is_char_boundary(s, 1))
    println(str.char_len(s, 0))
    println(str.compare("abc", "abd"))
    println(str.starts_with(s, "élé"))
    println(str.ends_with(s, "route"))
    println(str.index_of(s, "sur"))
    println(str.contains(s, "la"))
    println(str.trim("  padded  "))
    println(str.substring(s, 0, 3))
    let parts = str.split("a,b,c", ",")
    println(parts.len)
    println(str.join(&parts, "-"))
    println(str.repeat("ab", 3))
    match str.parse_int("-4321") {
        Result::Ok(v) => println(v)
        Result::Err(e) => println(e)
    }
    let b = Box{s: "field"}
    let fv: &[u8] = b.s.bytes
    let sv: &[u8] = s.bytes
    println(churn(300))
    println(fv[0] as i64)
    println(sv[0] as i64)
    io.write_str(1, "io\n")
    io.write_out("out\n".bytes)
    io.print_out("print\n")
    return 0
}
"#;

#[test]
fn s11_3_every_legitimate_bytes_shape_in_std_still_compiles_and_runs() {
    let h = Harness::new();
    h.value_row(
        "S1.1-3",
        S11_STD_BYTES_SHAPES,
        "21\nfalse\n2\n-1\ntrue\ntrue\n9\ntrue\npadded\nélé\n3\na-b-c\nababab\n-4321\n1791\n102\n\
         195\nio\nout\nprint\n",
    );
    h.assert_legs(8);
}

const S11_AS_SLICE_OF_A_TEMPORARY: &str = r#"
fn mkvec() -> Vec<i64> {
    let mut v: Vec<i64> = Vec::new()
    Vec::push(v, 7)
    return v
}

fn main() -> i64 {
    let s: &[i64] = Vec::as_slice(mkvec())
    println(s[0])
    return 0
}
"#;

#[test]
fn s11_4_the_vec_as_slice_control_stays_refused() {
    let h = Harness::new();
    h.rejects_at_every_level(
        "S1.1-4",
        S11_AS_SLICE_OF_A_TEMPORARY,
        "E0421",
        "[no-place] the receiver of Vec::as_slice denotes no storage",
    );
    h.assert_legs(4);
}

const S11_LENGTH_OF_A_VIEW_OF_A_TEMPORARY: &str = r#"
fn mkheap(n: i64) -> string {
    return string::substring_bytes("ABCDEFGH", 0, n)
}

fn main() -> i64 {
    println(mkheap(4).bytes.len)
    return 0
}
"#;

#[test]
fn s11_5_the_air_guard_fires_when_the_surface_refusal_is_bypassed() {
    let h = Harness::new();
    let mut rendered = Vec::new();
    for (tag, src) in [
        ("value", S11_VIEW_OF_A_TEMPORARY),
        ("argument", S11_VIEW_OF_A_TEMPORARY_IN_AN_ARGUMENT),
        ("length", S11_LENGTH_OF_A_VIEW_OF_A_TEMPORARY),
    ] {
        let root = h.stage("S1.1-5", tag, src);
        aelys_sema::ablation::set_bytes_no_place_unchecked(true);
        let ablated = lower_file_to_air(&root, OptimizationLevel::None);
        aelys_sema::ablation::set_bytes_no_place_unchecked(false);
        let text = match ablated {
            Ok(_) => panic!(
                "S1.1-5 at {tag}: with the surface refusal ablated the program lowered clean, so \
                 the AIR guard is asserted by nothing"
            ),
            Err(rendered) => rendered,
        };
        assert!(
            text.contains("[E0901]") || text.contains("error[E0901]"),
            "S1.1-5 at {tag}: the bypassed refusal MUST become a compiler fault\n{text}"
        );
        assert!(
            text.contains("sema must reject it (E0421)"),
            "S1.1-5 at {tag}: the fault MUST name the refusal it stands behind\n{text}"
        );
        rendered.push((tag, text));

        // the positive control: with the seam off the same program is refused at the surface
        let repaired = match lower_file_to_air(&root, OptimizationLevel::None) {
            Ok(_) => panic!("S1.1-5 at {tag}: the unablated compiler MUST refuse it"),
            Err(rendered) => rendered,
        };
        assert!(
            repaired.contains("[E0421]") && repaired.contains(NO_PLACE_SAYS),
            "S1.1-5 at {tag}: with the seam off this is a surface refusal, not a fault\n{repaired}"
        );
    }
    for wanted in ["reached BIR building", "reached AIR lowering"] {
        assert!(
            rendered.iter().any(|(_, t)| t.contains(wanted)),
            "S1.1-5: no row reached the guard saying {wanted:?}; both `.bytes` handlers must be \
             witnessed firing\n{rendered:?}"
        );
    }
}

#[test]
fn s11_6_the_e0421_explanation_names_the_new_producer_and_its_one_carve_out() {
    let rows = aelys_common::diagnostic::registry::all_codes();
    let row = rows
        .iter()
        .find(|r| r.code == "E0421")
        .expect("E0421 must have a registry row");
    for wanted in [
        "the receiver of\n`.bytes`",
        "make_string().bytes           // E0421",
        "A string literal is the one receiver `.bytes` accepts without a",
    ] {
        assert!(
            row.explanation.contains(wanted),
            "S1.1-6: `aelys --explain E0421` must say {wanted:?}; a refusal the user meets and \
             the explanation does not mention reads as a compiler bug\n{}",
            row.explanation
        );
    }
}

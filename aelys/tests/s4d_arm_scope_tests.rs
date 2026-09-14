use aelys_air::{AirStmtKind, Operand, Place, Rvalue};
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
        let mut by_level: Vec<(&str, String)> = Vec::new();
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
                    "{id} at {tag}/{alloc_name}: stdout MUST be {stdout:?}\n{src}\nstderr:\n{seen_err}"
                );
                by_level.push((tag, seen_out));
            }
        }
        let (first_tag, first) = &by_level[0];
        for (tag, seen) in &by_level[1..] {
            assert_eq!(
                seen, first,
                "{id}: {tag} prints {seen:?} where {first_tag} prints {first:?}; a value that \
                 reads the optimizer is not a value\n{src}"
            );
        }
    }
}

const S4D_ARM_SHADOWS_OUTER: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let o: Option<i64> = Option::Some(7)
    let t: i64 = 5
    match o {
        Option::Some(t) => println(t),
        Option::None => println(0),
    }
    println(t)
    return 0
}
"#;

const S4D_SIBLING_ARM: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let o: Option<i64> = Option::None
    let t: i64 = 5
    match o {
        Option::Some(t) => println(t),
        Option::None => println(t),
    }
    return 0
}
"#;

const S4D_ARM_SHADOWS_PARAM: &str = r#"
needs Option from std.result

fn f(t: i64, o: Option<i64>) -> i64 {
    match o {
        Option::Some(t) => println(t),
        Option::None => println(0),
    }
    return t
}

fn main() -> i64 {
    println(f(5, Option::Some(7)))
    return 0
}
"#;

const S4D_NESTED_MATCH: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let a: Option<i64> = Option::Some(1)
    let b: Option<i64> = Option::Some(2)
    match a {
        Option::Some(t) => {
            match b {
                Option::Some(t) => println(t),
                Option::None => println(0),
            }
            println(t)
        }
        Option::None => println(9),
    }
    return 0
}
"#;

const S4D_CATCH_BINDER: &str = r#"
enum Result<T, E> { Ok(T), Err(E) }
enum E { X }

fn get_err() -> Result<i64, E> { return Result::Err(E::X) }

fn main() -> i64 {
    let e: i64 = 5
    let v: i64 = get_err() catch |e| 100
    println(v)
    println(e)
    return 0
}
"#;

const S4D_MATCH_IN_A_LOOP: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let t: i64 = 5
    for i in 0..2 {
        let o: Option<i64> = Option::Some(i + 1)
        match o {
            Option::Some(t) => println(t),
            Option::None => println(0),
        }
        println(t)
    }
    return 0
}
"#;

// the outer value comes from a call, so no level can constant-fold the read back to 5
const S4D_OUTER_IS_NOT_A_CONSTANT: &str = r#"
needs Option from std.result

fn five() -> i64 { return 5 }

fn main() -> i64 {
    let o: Option<i64> = Option::Some(7)
    let t: i64 = five()
    match o {
        Option::Some(t) => println(t),
        Option::None => println(0),
    }
    println(t)
    return 0
}
"#;

const S4D_CAPTURED_AFTER_THE_ARM: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let t: i64 = 5
    let o: Option<i64> = Option::Some(7)
    match o {
        Option::Some(t) => println(t),
        Option::None => println(0),
    }
    let f = fn() -> i64 { return t }
    println(f())
    return 0
}
"#;

const S4D_MATCH_INSIDE_A_LAMBDA: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let f = fn(t: i64, o: Option<i64>) -> i64 {
        match o {
            Option::Some(t) => println(t),
            Option::None => println(0),
        }
        return t
    }
    println(f(5, Option::Some(7)))
    return 0
}
"#;

const S4D_TWO_MATCHES_IN_A_ROW: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let t: i64 = 5
    let a: Option<i64> = Option::Some(7)
    let b: Option<i64> = Option::None
    match a {
        Option::Some(t) => println(t),
        Option::None => println(0),
    }
    match b {
        Option::Some(u) => println(u),
        Option::None => println(t),
    }
    return 0
}
"#;

// an enclosing block truncated this one anyway, so it is a control and not a witness
const S4D_ARM_INSIDE_AN_IF_CONTROL: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let t: i64 = 5
    let o: Option<i64> = Option::Some(7)
    if t > 0 {
        match o {
            Option::Some(t) => println(t),
            Option::None => println(0),
        }
    }
    println(t)
    return 0
}
"#;

const S4D_LET_INSIDE_THE_ARM_CONTROL: &str = r#"
needs Option from std.result

fn main() -> i64 {
    let t: i64 = 5
    let o: Option<i64> = Option::Some(7)
    match o {
        Option::Some(u) => {
            let t: i64 = u + 1
            println(t)
        }
        Option::None => println(0),
    }
    println(t)
    return 0
}
"#;

const S4D_LAMBDA_PARAM_CONTROL: &str = r#"
fn main() -> i64 {
    let t: i64 = 5
    let f = fn(t: i64) -> i64 { return t + 1 }
    println(f(9))
    println(t)
    return 0
}
"#;

const S4D_BLOCK_LET_CONTROL: &str = r#"
fn g(t: i64) -> i64 {
    {
        let t: i64 = 9
        println(t)
    }
    return t
}

fn main() -> i64 {
    println(g(5))
    return 0
}
"#;

const S4D_FOR_ITERATOR_CONTROL: &str = r#"
fn main() -> i64 {
    let i: i64 = 5
    for i in 0..2 {
        println(i)
    }
    println(i)
    return 0
}
"#;

const S4D_FOREACH_CONTROL: &str = r#"
fn main() -> i64 {
    let x: i64 = 5
    let v: [i64; 2] = [7, 8]
    for x in v {
        println(x)
    }
    println(x)
    return 0
}
"#;

const S4D_WHILE_BODY_CONTROL: &str = r#"
fn main() -> i64 {
    let t: i64 = 5
    let mut n: i64 = 0
    while n < 2 {
        let t: i64 = 9
        println(t)
        n = n + 1
    }
    println(t)
    return 0
}
"#;

fn one_row(id: &str, src: &str, stdout: &str) {
    let h = Harness::new();
    h.value_row(id, src, stdout);
    assert_legs(&h, 8);
}

#[test]
fn s4d_1a_an_arm_binder_does_not_shadow_the_outer_local_after_the_arm() {
    one_row("s4d_1a_outer_local", S4D_ARM_SHADOWS_OUTER, "7\n5\n");
}

#[test]
fn s4d_1b_an_arm_does_not_see_the_binder_of_the_arm_before_it() {
    one_row("s4d_1b_sibling_arm", S4D_SIBLING_ARM, "5\n");
}

#[test]
fn s4d_1c_an_arm_binder_does_not_shadow_a_parameter_after_the_arm() {
    one_row("s4d_1c_parameter", S4D_ARM_SHADOWS_PARAM, "7\n5\n");
}

#[test]
fn s4d_1d_an_inner_arm_binder_does_not_outlive_the_inner_match() {
    one_row("s4d_1d_nested_match", S4D_NESTED_MATCH, "2\n1\n");
}

#[test]
fn s4d_1e_a_catch_binder_is_the_same_door() {
    one_row("s4d_1e_catch_binder", S4D_CATCH_BINDER, "100\n5\n");
}

#[test]
fn s4d_1f_an_arm_binder_in_a_loop_dies_on_every_iteration() {
    one_row("s4d_1f_in_a_loop", S4D_MATCH_IN_A_LOOP, "1\n5\n2\n5\n");
}

#[test]
fn s4d_1g_the_outer_value_no_level_can_fold() {
    one_row(
        "s4d_1g_unfoldable_outer",
        S4D_OUTER_IS_NOT_A_CONSTANT,
        "7\n5\n",
    );
}

#[test]
fn s4d_1h_a_closure_created_after_the_arm_captures_the_outer_local() {
    one_row("s4d_1h_capture", S4D_CAPTURED_AFTER_THE_ARM, "7\n5\n");
}

#[test]
fn s4d_1i_a_match_inside_a_lambda_does_not_rebind_its_parameter() {
    one_row("s4d_1i_in_a_lambda", S4D_MATCH_INSIDE_A_LAMBDA, "7\n5\n");
}

#[test]
fn s4d_1j_the_next_match_does_not_inherit_the_previous_ones_binder() {
    one_row("s4d_1j_two_matches", S4D_TWO_MATCHES_IN_A_ROW, "7\n5\n");
}

#[test]
fn s4d_2_the_scopes_that_already_popped_still_pop() {
    let h = Harness::new();
    for (id, src, stdout) in [
        ("s4d_2a_lambda_param", S4D_LAMBDA_PARAM_CONTROL, "10\n5\n"),
        ("s4d_2b_block_let", S4D_BLOCK_LET_CONTROL, "9\n5\n"),
        ("s4d_2c_for_iterator", S4D_FOR_ITERATOR_CONTROL, "0\n1\n5\n"),
        ("s4d_2d_foreach", S4D_FOREACH_CONTROL, "7\n8\n5\n"),
        ("s4d_2e_while_body", S4D_WHILE_BODY_CONTROL, "9\n9\n5\n"),
        ("s4d_2f_arm_in_an_if", S4D_ARM_INSIDE_AN_IF_CONTROL, "7\n5\n"),
        ("s4d_2g_let_in_the_arm", S4D_LET_INSIDE_THE_ARM_CONTROL, "8\n5\n"),
    ] {
        h.value_row(id, src, stdout);
    }
    assert_legs(&h, 56);
}

#[test]
fn s4d_3_the_payload_local_is_named_only_inside_its_own_arm() {
    let h = Harness::new();
    for (tag, opt) in LEVELS {
        let root = h.stage("s4d_3_air", tag, S4D_ARM_SHADOWS_OUTER);
        let air = lower_file_to_air(&root, *opt)
            .unwrap_or_else(|e| panic!("s4d_3 at {tag} must lower:\n{e}"));
        let main = air
            .functions
            .iter()
            .find(|f| f.name == "main")
            .expect("the program has a main");

        let mut payload_locals: Vec<u32> = Vec::new();
        for block in &main.blocks {
            for stmt in &block.stmts {
                if let AirStmtKind::Assign {
                    place: Place::Local(local),
                    rvalue: Rvalue::EnumPayload { .. },
                } = &stmt.kind
                {
                    payload_locals.push(local.0);
                }
            }
        }
        assert_eq!(
            payload_locals.len(),
            1,
            "s4d_3 at {tag}: this program binds exactly one arm payload"
        );
        let binder = payload_locals[0];

        for block in &main.blocks {
            let defines_binder = block.stmts.iter().any(|s| {
                matches!(&s.kind, AirStmtKind::Assign { place: Place::Local(l), .. } if l.0 == binder)
            });
            if defines_binder {
                continue;
            }
            for stmt in &block.stmts {
                if let AirStmtKind::CallVoid { args, .. } = &stmt.kind {
                    for arg in args {
                        let named = matches!(arg, Operand::Copy(l) | Operand::Move(l) if l.0 == binder);
                        assert!(
                            !named,
                            "s4d_3 at {tag}: block {} reads the arm binder %{binder} outside the \
                             arm that bound it",
                            block.id.0
                        );
                    }
                }
            }
        }
    }
}

fn assert_legs(h: &Harness, expected: usize) {
    if h.linker_skips.get() > 0 && linker_skip_declared() {
        return;
    }
    assert_eq!(
        h.legs.get(),
        expected,
        "this test must execute exactly {expected} legs; a leg that silently stopped running \
         is a confident zero"
    );
}

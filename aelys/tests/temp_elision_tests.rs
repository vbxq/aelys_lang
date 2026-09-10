use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::process::Command;
use tempfile::tempdir;

mod common;
use common::{exe_path_for, linker_unavailable};

struct Run {
    code: i32,
    stdout: String,
    stderr: String,
}

fn run_at(src: &str, opt: OptimizationLevel, alloc: &str) -> Option<Run> {
    let _pin = common::pin_legs("temp_elision run", 1);
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(&source_path, opt, false, RuntimeVariant::Rc) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                common::require_linker_skip("a skipped counter row carries no runtime evidence");
                return None;
            }
            panic!("compilation/link should succeed: {err}");
        }
    }

    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        common::require_linker_skip("a skipped counter row carries no runtime evidence");
        return None;
    }

    common::note_leg();
    let output = Command::new(&exe)
        .env("AELYS_RC_STATS", "1")
        .env("AELYS_ALLOC", alloc)
        .output()
        .expect("run compiled exe");

    Some(Run {
        code: output.status.code().expect("exit code"),
        stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
    })
}

fn parse_counters(stderr: &str, tag: &str) -> Option<(i64, i64)> {
    let prefix = format!("[{tag}] allocs=");
    let line = stderr.lines().find(|l| l.trim().starts_with(&prefix))?;
    let rest = line.trim().strip_prefix(&prefix)?;
    let (allocs, frees) = rest.split_once(" frees=")?;
    Some((allocs.trim().parse().ok()?, frees.trim().parse().ok()?))
}

fn managed(stderr: &str) -> Option<(i64, i64)> {
    parse_counters(stderr, "rc")
}

fn raw(stderr: &str) -> Option<(i64, i64)> {
    parse_counters(stderr, "raw")
}

const LEVELS: [(OptimizationLevel, &str); 2] = [
    (OptimizationLevel::None, "-O0"),
    (OptimizationLevel::Standard, "-O2"),
];
const ALLOCATORS: [&str; 2] = ["immix", "malloc"];

fn for_every_shape(src: &str, check: impl Fn(&Run, &str)) {
    for (opt, opt_name) in LEVELS {
        for alloc in ALLOCATORS {
            if let Some(run) = run_at(src, opt, alloc) {
                check(&run, &format!("{opt_name} AELYS_ALLOC={alloc}"));
            }
        }
    }
}

const HELLO: &str = r#"
fn main() -> i64 {
    println("hello")
    return 0
}
"#;

const ONE_INTERPOLATION: &str = r#"
fn main() -> i64 {
    let n = 7
    println("{n}")
    return 0
}
"#;

const THREE_INTERPOLATIONS: &str = r#"
fn main() -> i64 {
    let n = 7
    println("{n}")
    println("{n}")
    println("{n}")
    return 0
}
"#;

const ESCAPING_INTERPOLATION: &str = r#"
let mut k: str = ""

fn main() -> i64 {
    let n = 7
    k = "{n}"
    println(k)
    return 0
}
"#;

const UNBOUND_VEC: &str = r#"
fn main() -> i64 {
    return Vec::len(vec[1, 2])
}
"#;

const BOUND_VEC: &str = r#"
fn main() -> i64 {
    let v = vec[1, 2, 3]
    return Vec::len(v)
}
"#;

#[test]
fn raw_counter_reads_zero_without_interpolation() {
    for_every_shape(HELLO, |run, at| {
        assert_eq!(run.code, 0, "{at}: exits 0; stderr:\n{}", run.stderr);
        assert_eq!(run.stdout, "hello\n", "{at}");
        assert_eq!(
            raw(&run.stderr),
            Some((0, 0)),
            "{at}: a print with no interpolation raws nothing; stderr:\n{}",
            run.stderr
        );
        assert_eq!(managed(&run.stderr), Some((0, 0)), "{at}");
    });
}

#[test]
fn one_interpolation_into_a_print_allocates_nothing() {
    for_every_shape(ONE_INTERPOLATION, |run, at| {
        assert_eq!(run.code, 0, "{at}: exits 0; stderr:\n{}", run.stderr);
        assert_eq!(run.stdout, "7\n", "{at}: the value still prints");
        assert_eq!(
            raw(&run.stderr),
            Some((0, 0)),
            "{at}: the helper writes into the caller frame; stderr:\n{}",
            run.stderr
        );
        assert_eq!(managed(&run.stderr), Some((0, 0)), "{at}");
    });
}

#[test]
fn three_interpolations_stay_flat_instead_of_linear() {
    for_every_shape(THREE_INTERPOLATIONS, |run, at| {
        assert_eq!(run.code, 0, "{at}: exits 0; stderr:\n{}", run.stderr);
        assert_eq!(run.stdout, "7\n7\n7\n", "{at}");
        assert_eq!(
            raw(&run.stderr),
            Some((0, 0)),
            "{at}: three prints must not raw three buffers; stderr:\n{}",
            run.stderr
        );
    });
}

#[test]
fn an_escaping_interpolation_still_allocates_and_still_prints() {
    for_every_shape(ESCAPING_INTERPOLATION, |run, at| {
        assert_eq!(run.code, 0, "{at}: exits 0; stderr:\n{}", run.stderr);
        assert_eq!(
            run.stdout, "7\n",
            "{at}: a global outlives its statement and must still read back"
        );
        assert_eq!(
            raw(&run.stderr),
            Some((1, 0)),
            "{at}: the peephole must decline a value that flows into a global; stderr:\n{}",
            run.stderr
        );
    });
}

#[test]
fn the_raw_counter_discriminates_between_two_programs() {
    for (opt, opt_name) in LEVELS {
        let (Some(quiet), Some(loud)) = (
            run_at(HELLO, opt, "immix"),
            run_at(ESCAPING_INTERPOLATION, opt, "immix"),
        ) else {
            continue;
        };
        assert_eq!(raw(&quiet.stderr), Some((0, 0)), "{opt_name}");
        assert_eq!(raw(&loud.stderr), Some((1, 0)), "{opt_name}");
        assert_ne!(
            raw(&quiet.stderr),
            raw(&loud.stderr),
            "{opt_name}: an instrument that reads the same number for both measures nothing"
        );
    }
}

#[test]
fn an_unbound_vec_temporary_is_balanced() {
    for_every_shape(UNBOUND_VEC, |run, at| {
        assert_eq!(run.code, 2, "{at}: len is 2; stderr:\n{}", run.stderr);
        assert_eq!(
            managed(&run.stderr),
            Some((1, 1)),
            "{at}: nothing else will ever release the literal's buffer; stderr:\n{}",
            run.stderr
        );
    });
}

#[test]
fn a_bound_vec_keeps_its_single_release() {
    for_every_shape(BOUND_VEC, |run, at| {
        assert_eq!(run.code, 3, "{at}: len is 3; stderr:\n{}", run.stderr);
        assert_eq!(
            managed(&run.stderr),
            Some((1, 1)),
            "{at}: the binding took the share, releasing the temp too would double free; \
             stderr:\n{}",
            run.stderr
        );
    });
}

#[test]
fn a_loop_reuses_one_frame_buffer() {
    let src = r#"
fn main() -> i64 {
    let mut i = 0
    while i < 5 {
        println("{i}")
        i = i + 1
    }
    return 0
}
"#;
    for_every_shape(src, |run, at| {
        assert_eq!(run.code, 0, "{at}: exits 0; stderr:\n{}", run.stderr);
        assert_eq!(run.stdout, "0\n1\n2\n3\n4\n", "{at}");
        assert_eq!(
            raw(&run.stderr),
            Some((0, 0)),
            "{at}: the buffer is per frame, so the trip count cannot show up; stderr:\n{}",
            run.stderr
        );
    });
}

#[test]
fn a_vec_temporary_released_inside_a_loop_is_balanced() {
    let src = r#"
fn main() -> i64 {
    let mut i = 0
    let mut total = 0
    while i < 4 {
        total = total + Vec::len(vec[1, 2, 3])
        i = i + 1
    }
    return total
}
"#;
    for_every_shape(src, |run, at| {
        assert_eq!(run.code, 12, "{at}: 4 x 3; stderr:\n{}", run.stderr);
        assert_eq!(
            managed(&run.stderr),
            Some((4, 4)),
            "{at}: one release per trip, never two; stderr:\n{}",
            run.stderr
        );
    });
}

#[test]
fn a_returned_vec_temporary_is_not_released_by_its_maker() {
    let src = r#"
fn mk() -> Vec<i64> {
    return vec[1, 2]
}

fn main() -> i64 {
    let v = mk()
    return Vec::len(v)
}
"#;
    for_every_shape(src, |run, at| {
        assert_eq!(run.code, 2, "{at}: len is 2; stderr:\n{}", run.stderr);
        assert_eq!(
            managed(&run.stderr),
            Some((1, 1)),
            "{at}: the share left through the return, the caller owns the only release; \
             stderr:\n{}",
            run.stderr
        );
    });
}

#[test]
fn a_float_interpolation_takes_the_wider_frame_buffer() {
    let src = r#"
fn main() -> i64 {
    let f = 1.5
    println("{f}")
    return 0
}
"#;
    for_every_shape(src, |run, at| {
        assert_eq!(run.code, 0, "{at}: exits 0; stderr:\n{}", run.stderr);
        assert_eq!(run.stdout, "1.5\n", "{at}");
        assert_eq!(
            raw(&run.stderr),
            Some((0, 0)),
            "{at}: the f64 helper writes into the frame too; stderr:\n{}",
            run.stderr
        );
    });
}

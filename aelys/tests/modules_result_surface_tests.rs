use aelys_driver::{compile_file_with_llvm, lower_file_to_air};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
    ("-O3", OptimizationLevel::Aggressive),
];

type Files = &'static [(&'static str, &'static str)];

fn stage(files: Files) -> TempDir {
    let dir = tempdir().expect("tempdir");
    for (name, body) in files {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).expect("create fixture directory");
        }
        fs::write(&path, body).expect("write fixture");
    }
    dir
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

fn run_row(id: &str, files: Files, exit: i32) {
    assert!(
        (0..256).contains(&exit),
        "{id}: an expected exit of {exit} cannot be observed through an 8-bit status"
    );
    for (level, opt) in LEVELS {
        let dir = stage(files);
        let root = dir.path().join("root.aelys");
        if let Err(err) = compile_file_with_llvm(&root, *opt, false) {
            panic!("{id} at {level}: MUST compile and link\nerror:\n{err}");
        }
        let exe = exe_path_for(&root);
        assert!(exe.is_file(), "{id} at {level}: no executable was produced");
        let out = Command::new(&exe).output().expect("run executable");
        assert_eq!(
            exit_code(&out.status),
            exit,
            "{id} at {level}: the answer MUST be {exit}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
}

// one level only: the rejection is decided before the optimizer, so it cannot move with -o
fn reject_row(id: &str, files: Files, says: &[&str], absent: &[&str]) {
    let dir = stage(files);
    let root = dir.path().join("root.aelys");
    let rendered = match lower_file_to_air(&root, OptimizationLevel::None) {
        Ok(_) => panic!("{id}: MUST be rejected, but it was accepted"),
        Err(rendered) => rendered,
    };
    for needle in says {
        assert!(
            rendered.contains(needle),
            "{id}: the diagnostic MUST say {needle:?}\nrendered:\n{rendered}"
        );
    }
    for needle in absent {
        assert!(
            !rendered.contains(needle),
            "{id}: the diagnostic MUST NOT contain {needle:?}\nrendered:\n{rendered}"
        );
    }
}

const RES: &str = "\
pub enum Result<T, E> { Ok(T), Err(E) }
pub enum Option<T> { Some(T), None }

pub fn half(n: i64) -> Result<i64, i64> {
    if n % 2 == 0 { return Result::Ok(n / 2) }
    return Result::Err(n)
}

pub fn safe(n: i64) -> Result<i64, Never> {
    return Result::Ok(n)
}

pub fn maybe(n: i64) -> Option<i64> {
    if n > 0 { return Option::Some(n) }
    return Option::None
}
";

#[test]
fn question_on_an_imported_result_named_import_shape() {
    run_row(
        "MRS-1a",
        &[
            (
                "root.aelys",
                "\
needs Result from res
needs res

fn run(n: i64) -> Result<i64, i64> {
    let v: i64 = res.half(n)?
    return Result::Ok(v + 1)
}

fn main() -> i64 {
    return match run(8) {
        Result::Ok(v) => v
        Result::Err(e) => 90 + e
    }
}
",
            ),
            ("res.aelys", RES),
        ],
        5,
    );
}

#[test]
fn question_on_an_imported_result_namespace_shape() {
    run_row(
        "MRS-1b",
        &[
            (
                "root.aelys",
                "\
needs res

fn run(n: i64) -> res.Result<i64, i64> {
    let v: i64 = res.half(n)?
    return res.Result::Ok(v + 1)
}

fn main() -> i64 {
    return match run(7) {
        res.Result::Ok(v) => v
        res.Result::Err(e) => 90 + e
    }
}
",
            ),
            ("res.aelys", RES),
        ],
        97,
    );
}

#[test]
fn question_on_an_imported_option_named_import_shape() {
    run_row(
        "MRS-2a",
        &[
            (
                "root.aelys",
                "\
needs Option from res
needs res

fn run(n: i64) -> Option<i64> {
    let v: i64 = res.maybe(n)?
    return Option::Some(v + 1)
}

fn main() -> i64 {
    return match run(7) {
        Option::Some(v) => v
        Option::None => 90
    }
}
",
            ),
            ("res.aelys", RES),
        ],
        8,
    );
}

#[test]
fn question_on_an_imported_option_namespace_shape() {
    run_row(
        "MRS-2b",
        &[
            (
                "root.aelys",
                "\
needs res

fn run(n: i64) -> res.Option<i64> {
    let v: i64 = res.maybe(n)?
    return res.Option::Some(v + 1)
}

fn main() -> i64 {
    return match run(0) {
        res.Option::Some(v) => v
        res.Option::None => 90
    }
}
",
            ),
            ("res.aelys", RES),
        ],
        90,
    );
}

#[test]
fn catch_on_an_imported_result_passes_ok_through() {
    run_row(
        "MRS-3a",
        &[
            (
                "root.aelys",
                "\
needs res

fn main() -> i64 {
    return res.half(8) catch |e| 90
}
",
            ),
            ("res.aelys", RES),
        ],
        4,
    );
}

#[test]
fn catch_on_an_imported_result_runs_the_handler_on_err() {
    run_row(
        "MRS-3b",
        &[
            (
                "root.aelys",
                "\
needs res

fn main() -> i64 {
    return res.half(9) catch |e| e + 1
}
",
            ),
            ("res.aelys", RES),
        ],
        10,
    );
}

#[test]
fn unwrap_on_an_imported_result() {
    run_row(
        "MRS-4a",
        &[
            (
                "root.aelys",
                "needs res\n\nfn main() -> i64 {\n    return res.half(8).unwrap()\n}\n",
            ),
            ("res.aelys", RES),
        ],
        4,
    );
}

#[test]
fn expect_on_an_imported_result() {
    run_row(
        "MRS-4b",
        &[
            (
                "root.aelys",
                "needs res\n\nfn main() -> i64 {\n    return res.half(10).expect(\"must be even\")\n}\n",
            ),
            ("res.aelys", RES),
        ],
        5,
    );
}

#[test]
fn map_error_on_an_imported_result() {
    run_row(
        "MRS-4c",
        &[
            (
                "root.aelys",
                "\
needs res
needs Result from res

fn widen(e: i64) -> i64 { return e + 1 }

fn main() -> i64 {
    let r: Result<i64, i64> = res.half(3).map_error(widen)
    return match r {
        Result::Ok(v) => 0
        Result::Err(e) => e
    }
}
",
            ),
            ("res.aelys", RES),
        ],
        4,
    );
}

#[test]
fn into_ok_on_an_imported_result() {
    run_row(
        "MRS-4d",
        &[
            (
                "root.aelys",
                "needs res\n\nfn main() -> i64 {\n    return res.safe(6).into_ok()\n}\n",
            ),
            ("res.aelys", RES),
        ],
        6,
    );
}

#[test]
fn unwrap_unchecked_on_an_imported_result() {
    run_row(
        "MRS-4e",
        &[
            (
                "root.aelys",
                "needs res\n\nfn main() -> i64 {\n    return unsafe { res.half(20).unwrap_unchecked() }\n}\n",
            ),
            ("res.aelys", RES),
        ],
        10,
    );
}

// the local result has ok at tag 1, so a lookup by the literal name stamps the wrong tag
#[test]
fn a_swapped_local_result_does_not_capture_an_imported_receiver() {
    run_row(
        "MRS-5",
        &[
            (
                "root.aelys",
                "\
needs res

enum Result<T, E> { Err(E), Ok(T) }

fn main() -> i64 {
    return res.half(8).unwrap()
}
",
            ),
            ("res.aelys", RES),
        ],
        4,
    );
}

#[test]
fn question_between_two_distinct_result_types_is_e0619() {
    reject_row(
        "MRS-6",
        &[
            (
                "root.aelys",
                "\
needs res

enum Result<T, E> { Ok(T), Err(E) }

fn run(n: i64) -> Result<i64, i64> {
    let v: i64 = res.half(n)?
    return Result::Ok(v)
}

fn main() -> i64 {
    return match run(8) {
        Result::Ok(v) => v
        Result::Err(e) => 90
    }
}
",
            ),
            ("res.aelys", RES),
        ],
        &["[E0619]", "res.Result<i64, i64>", "Result<i64, i64>"],
        &["[E0304]", "[E0620]"],
    );
}

#[test]
fn question_between_two_distinct_option_types_is_e0619() {
    reject_row(
        "MRS-6b",
        &[
            (
                "root.aelys",
                "\
needs res

enum Option<T> { Some(T), None }

fn run(n: i64) -> Option<i64> {
    let v: i64 = res.maybe(n)?
    return Option::Some(v)
}

fn main() -> i64 {
    return match run(8) {
        Option::Some(v) => v
        Option::None => 90
    }
}
",
            ),
            ("res.aelys", RES),
        ],
        &["[E0619]", "res.Option<i64>", "`Option`"],
        &["[E0304]", "[E0620]"],
    );
}

#[test]
fn three_modules_sharing_a_fourth_carrier_propagate_with_question() {
    run_row(
        "MRS-7",
        &[
            (
                "root.aelys",
                "\
needs Result from res
needs beta

fn run(n: i64) -> Result<i64, i64> {
    let v: i64 = beta.twice(n)?
    return Result::Ok(v + 10)
}

fn main() -> i64 {
    return match run(3) {
        Result::Ok(v) => v
        Result::Err(e) => 90 + e
    }
}
",
            ),
            (
                "beta.aelys",
                "\
needs Result from res
needs alpha

pub fn twice(n: i64) -> Result<i64, i64> {
    let x: i64 = alpha.bump(n)?
    let y: i64 = alpha.bump(x)?
    return Result::Ok(y)
}
",
            ),
            (
                "alpha.aelys",
                "\
needs Result from res

pub fn bump(n: i64) -> Result<i64, i64> {
    if n < 0 { return Result::Err(1) }
    return Result::Ok(n + 1)
}
",
            ),
            ("res.aelys", "pub enum Result<T, E> { Ok(T), Err(E) }\n"),
        ],
        15,
    );
}

#[test]
fn three_modules_sharing_a_fourth_carrier_propagate_the_error() {
    run_row(
        "MRS-7b",
        &[
            (
                "root.aelys",
                "\
needs Result from res
needs beta

fn run(n: i64) -> Result<i64, i64> {
    let v: i64 = beta.twice(n)?
    return Result::Ok(v + 10)
}

fn main() -> i64 {
    return match run(0 - 5) {
        Result::Ok(v) => v
        Result::Err(e) => 90 + e
    }
}
",
            ),
            (
                "beta.aelys",
                "\
needs Result from res
needs alpha

pub fn twice(n: i64) -> Result<i64, i64> {
    let x: i64 = alpha.bump(n)?
    let y: i64 = alpha.bump(x)?
    return Result::Ok(y)
}
",
            ),
            (
                "alpha.aelys",
                "\
needs Result from res

pub fn bump(n: i64) -> Result<i64, i64> {
    if n < 0 { return Result::Err(1) }
    return Result::Ok(n + 1)
}
",
            ),
            ("res.aelys", "pub enum Result<T, E> { Ok(T), Err(E) }\n"),
        ],
        91,
    );
}

#[test]
fn an_imported_result_in_statement_position_is_must_use() {
    reject_row(
        "MRS-8",
        &[
            (
                "root.aelys",
                "needs res\n\nfn main() -> i64 {\n    res.half(3)\n    return 0\n}\n",
            ),
            ("res.aelys", RES),
        ],
        &["[E0411]", "[must-use]"],
        &[],
    );
}

#[test]
fn a_discarded_imported_result_is_still_accepted() {
    run_row(
        "MRS-8b",
        &[
            (
                "root.aelys",
                "needs res\n\nfn main() -> i64 {\n    discard res.half(3)\n    return 0\n}\n",
            ),
            ("res.aelys", RES),
        ],
        0,
    );
}

#[test]
fn a_module_internal_result_in_statement_position_is_must_use() {
    reject_row(
        "MRS-9",
        &[
            (
                "root.aelys",
                "needs res\n\nfn main() -> i64 {\n    return res.touch(3)\n}\n",
            ),
            (
                "res.aelys",
                "\
pub enum Result<T, E> { Ok(T), Err(E) }

pub fn half(n: i64) -> Result<i64, i64> {
    if n % 2 == 0 { return Result::Ok(n / 2) }
    return Result::Err(n)
}

pub fn touch(n: i64) -> i64 {
    half(n)
    return 0
}
",
            ),
        ],
        &["[E0411]", "[must-use]", "res.aelys:9"],
        &[],
    );
}

// a behaviour change: the root never names beta, and this program compiled and exited 0 before
#[test]
fn a_transitively_reached_module_result_in_statement_position_is_must_use() {
    reject_row(
        "MRS-10",
        &[
            (
                "root.aelys",
                "needs alpha\n\nfn main() -> i64 {\n    return alpha.go(3)\n}\n",
            ),
            (
                "alpha.aelys",
                "needs beta\n\npub fn go(n: i64) -> i64 {\n    return beta.touch(n)\n}\n",
            ),
            (
                "beta.aelys",
                "\
pub enum Result<T, E> { Ok(T), Err(E) }
pub fn mk(n: i64) -> Result<i64, i64> { return Result::Ok(n) }
pub fn touch(n: i64) -> i64 {
    mk(n)
    return n
}
",
            ),
        ],
        &["[E0411]", "[must-use]", "beta.aelys:4"],
        &[],
    );
}

// a non regression companion, not a protection: must-use never matched an imported option, before or after
#[test]
fn an_imported_option_in_statement_position_is_not_must_use() {
    run_row(
        "MRS-11",
        &[
            (
                "root.aelys",
                "needs res\n\nfn main() -> i64 {\n    res.maybe(3)\n    return 0\n}\n",
            ),
            ("res.aelys", RES),
        ],
        0,
    );
}

#[test]
fn e0619_is_registered_and_the_explain_names_the_two_declarations() {
    let info = aelys_common::diagnostic::registry::lookup("E0619").expect("E0619 is registered");
    assert!(
        info.explanation.contains("two different nominal types"),
        "MRS-12: the explain must name what makes the two carriers distinct: {}",
        info.explanation
    );
    assert!(
        info.explanation.contains("needs Result from res"),
        "MRS-12: the explain must name the way out: {}",
        info.explanation
    );
}

#[test]
fn e0620_is_registered_and_the_explain_covers_the_error_handling_family() {
    let info = aelys_common::diagnostic::registry::lookup("E0620").expect("E0620 is registered");
    for needle in [
        "`?`",
        "`catch`",
        "`.unwrap_unchecked()`",
        "`.into_ok()`",
        "`.map_error(f)`",
    ] {
        assert!(
            info.explanation.contains(needle),
            "MRS-13: the explain must cover {needle}: {}",
            info.explanation
        );
    }
    assert!(
        info.explanation.contains("reported as E0619"),
        "MRS-13: the explain must send the carrier mismatch case to its own code: {}",
        info.explanation
    );
}

use aelys_driver::{
    LinkRequirement, RuntimeVariant, SourceOptions, compile_file_with_llvm_sources,
    lower_file_to_air_with_sources,
};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::{TempDir, tempdir};

const LEVELS: &[(&str, OptimizationLevel)] = &[
    ("-O0", OptimizationLevel::None),
    ("-O2", OptimizationLevel::Standard),
];

type Files<'a> = &'a [(&'a str, &'a str)];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

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

fn run_row(id: &str, files: Files, sources: &SourceOptions, exit: i32, stdout: &str) {
    assert!(
        (0..256).contains(&exit),
        "{id}: an expected exit of {exit} cannot be observed through an 8-bit status"
    );
    for (level, opt) in LEVELS {
        let dir = stage(files);
        let root = dir.path().join("root.aelys");
        if let Err(err) = compile_file_with_llvm_sources(
            &root,
            *opt,
            false,
            RuntimeVariant::Rc,
            &LinkRequirement::default(),
            sources,
        ) {
            panic!("{id} at {level}: MUST compile and link\nerror:\n{err}");
        }
        let exe = exe_path_for(&root);
        assert!(exe.is_file(), "{id} at {level}: no executable was produced");
        let out = Command::new(&exe).output().expect("run executable");
        assert_eq!(
            exit_code(&out.status),
            exit,
            "{id} at {level}: the answer MUST be {exit}\nstdout: {:?}\nstderr:\n{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            stdout,
            "{id} at {level}: stdout MUST be {stdout:?}"
        );
    }
}

fn reject_row(id: &str, files: Files, sources: &SourceOptions, code: &str, needles: &[&str]) {
    for (level, opt) in LEVELS {
        let dir = stage(files);
        let root = dir.path().join("root.aelys");
        let rendered = match lower_file_to_air_with_sources(&root, *opt, sources) {
            Ok(_) => panic!("{id} at {level}: MUST be rejected, but it was accepted"),
            Err(rendered) => rendered,
        };
        assert!(
            rendered.contains(code),
            "{id} at {level}: the diagnostic MUST carry {code}\nrendered:\n{rendered}"
        );
        for needle in needles {
            assert!(
                rendered.contains(needle),
                "{id} at {level}: the diagnostic MUST say {needle:?}\nrendered:\n{rendered}"
            );
        }
    }
}

const PRINTS: &str = r#"
needs std.io

nogc fn banner() -> i64 {
    return io.print_out("héllo, wörld ✓\n")
}

fn main() -> i64 {
    return banner()
}
"#;

// 19 bytes for 15 characters, so a witness that only printed ascii could not tell the two apart
#[test]
fn s31_a_program_that_imports_a_library_prints_a_multibyte_string() {
    run_row(
        "S3.1-1",
        &[("root.aelys", PRINTS)],
        &SourceOptions::with_include(vec![repo_root()]),
        19,
        "héllo, wörld ✓\n",
    );
}

const ABS_7: &str = r#"
needs std.math

fn main() -> i64 {
    return math.abs(0 - 7)
}
"#;

const LOCAL_MATH: &str = "pub nogc fn abs(x: i64) -> i64 {\n    return 99\n}\n";
const ROOT_MATH: &str = "pub nogc fn abs(x: i64) -> i64 {\n    return 7\n}\n";

#[test]
fn s32_a_module_resolves_from_an_include_root() {
    let lib = stage(&[("std/math.aelys", ROOT_MATH)]);
    run_row(
        "S3.2-1",
        &[("root.aelys", ABS_7)],
        &SourceOptions::with_include(vec![lib.path().to_path_buf()]),
        7,
        "",
    );
}

#[test]
fn s32_the_root_files_own_directory_outranks_every_include_root() {
    let a = stage(&[("std/math.aelys", ROOT_MATH)]);
    let b = stage(&[("std/math.aelys", ROOT_MATH)]);
    run_row(
        "S3.2-2",
        &[("root.aelys", ABS_7), ("std/math.aelys", LOCAL_MATH)],
        &SourceOptions::with_include(vec![a.path().to_path_buf(), b.path().to_path_buf()]),
        99,
        "",
    );
}

#[test]
fn s32_a_module_under_two_include_roots_is_refused_and_names_every_root() {
    let a = stage(&[("std/math.aelys", ROOT_MATH)]);
    let b = stage(&[("std/math.aelys", ROOT_MATH)]);
    let c = stage(&[("std/math.aelys", ROOT_MATH)]);
    let roots = vec![
        a.path().to_path_buf(),
        b.path().to_path_buf(),
        c.path().to_path_buf(),
    ];
    let named: Vec<String> = roots.iter().map(|r| r.display().to_string()).collect();
    let needles: Vec<&str> = named.iter().map(String::as_str).collect();
    let mut all = vec!["resolves in 3 search roots"];
    all.extend(needles);
    reject_row(
        "S3.2-3",
        &[("root.aelys", ABS_7)],
        &SourceOptions::with_include(roots),
        "[E0622]",
        &all,
    );
}

#[test]
fn s32_one_root_written_twice_is_not_ambiguous() {
    let a = stage(&[("std/math.aelys", ROOT_MATH)]);
    run_row(
        "S3.2-4",
        &[("root.aelys", ABS_7)],
        &SourceOptions::with_include(vec![
            a.path().to_path_buf(),
            a.path().to_path_buf(),
            a.path().join("."),
        ]),
        7,
        "",
    );
}

const NEEDS_NOPE: &str = r#"
needs std.nope

fn main() -> i64 {
    return 0
}
"#;

#[test]
fn s32_a_missing_module_names_every_candidate_it_tried() {
    let a = stage(&[("std/math.aelys", ROOT_MATH)]);
    let b = stage(&[("std/math.aelys", ROOT_MATH)]);
    let first = a
        .path()
        .join("std")
        .join("nope.aelys")
        .display()
        .to_string();
    let second = b
        .path()
        .join("std")
        .join("nope.aelys")
        .display()
        .to_string();
    reject_row(
        "S3.2-5",
        &[("root.aelys", NEEDS_NOPE)],
        &SourceOptions::with_include(vec![a.path().to_path_buf(), b.path().to_path_buf()]),
        "[E0601]",
        &["searched in:", &first, &second],
    );
}

#[test]
fn s32_a_library_module_that_imports_a_sibling_compiles_on_its_own() {
    let slice = repo_root().join("std").join("slice.aelys");
    let bare =
        lower_file_to_air_with_sources(&slice, OptimizationLevel::None, &SourceOptions::default());
    let Err(rendered) = bare else {
        panic!(
            "S3.2-6: with no search root `std/slice.aelys` MUST still look for \
             `std/std/result.aelys`"
        );
    };
    assert!(
        rendered.contains("[E0601]") && rendered.contains("std.result"),
        "S3.2-6: the bare rejection MUST be the module-not-found it always was\n{rendered}"
    );

    if let Err(err) = lower_file_to_air_with_sources(
        &slice,
        OptimizationLevel::None,
        &SourceOptions::with_include(vec![repo_root()]),
    ) {
        panic!("S3.2-6: with the library root on the search path it MUST compile\n{err}");
    }
}

const PRELUDE_OPTION: &str = r#"
fn main() -> i64 {
    let o: Option<i64> = Option::Some(41)
    return match o {
        Option::Some(v) => v + 1,
        Option::None => 0,
    }
}
"#;

#[test]
fn s33_a_program_with_no_needs_at_all_reads_option_from_the_prelude() {
    run_row(
        "S3.3-1",
        &[("root.aelys", PRELUDE_OPTION)],
        &SourceOptions::with_include(vec![repo_root()]),
        42,
        "",
    );
}

#[test]
fn s33_no_prelude_takes_the_name_back_out_of_scope() {
    let mut sources = SourceOptions::with_include(vec![repo_root()]);
    sources.prelude = None;
    reject_row(
        "S3.3-2",
        &[("root.aelys", PRELUDE_OPTION)],
        &sources,
        "[E0301]",
        &["unknown type 'Option'"],
    );
}

const OWN_OPTION: &str = r#"
enum Option<T> { Some(T), None, Both(T, T) }

fn main() -> i64 {
    let o: Option<i64> = Option::Both(20, 22)
    return match o {
        Option::Some(v) => v,
        Option::None => 0,
        Option::Both(a, b) => a + b,
    }
}
"#;

#[test]
fn s33_a_program_that_defines_its_own_option_keeps_it_with_no_diagnostic() {
    let sources = SourceOptions::with_include(vec![repo_root()]);
    run_row("S3.3-3", &[("root.aelys", OWN_OPTION)], &sources, 42, "");

    let dir = stage(&[("root.aelys", OWN_OPTION)]);
    let root = dir.path().join("root.aelys");
    let warnings = compile_file_with_llvm_sources(
        &root,
        OptimizationLevel::None,
        false,
        RuntimeVariant::Rc,
        &LinkRequirement::default(),
        &sources,
    )
    .expect("S3.3-3: the shadowing program MUST compile");
    assert!(
        warnings.is_empty(),
        "S3.3-3: shadowing the prelude MUST be silent, got {warnings:?}"
    );
}

const OWN_OPTION_MODULE: &str = "pub enum Option<T> { Some(T), None, Triple(T, T, T) }\n";

const IMPORTED_OPTION: &str = r#"
needs Option from mine

fn main() -> i64 {
    let o: Option<i64> = Option::Triple(10, 14, 18)
    return match o {
        Option::Some(v) => v,
        Option::None => 0,
        Option::Triple(a, b, c) => a + b + c,
    }
}
"#;

#[test]
fn s33_an_explicit_import_of_the_name_outranks_the_prelude() {
    run_row(
        "S3.3-4",
        &[
            ("root.aelys", IMPORTED_OPTION),
            ("mine.aelys", OWN_OPTION_MODULE),
        ],
        &SourceOptions::with_include(vec![repo_root()]),
        42,
        "",
    );
}

const EXPLICIT_RESULT: &str = r#"
needs std.result

fn main() -> i64 {
    let a: result.Result<i64, i64> = result.Result::Ok(7)
    let b: result.Result<i64, i64> = result.Result::Err(3)
    let s: result.Option<i64> = result.Option::Some(9)
    let n: result.Option<i64> = result.Option::None
    if result.is_ok(a) { println(1) } else { println(0) }
    if result.is_err(b) { println(1) } else { println(0) }
    if result.is_some(s) { println(1) } else { println(0) }
    if result.is_none(n) { println(1) } else { println(0) }
    return result.unwrap_or(a, -1) + result.some_or(s, -3)
}
"#;

const RESULT_LIB: &str = "std/result.aelys";

#[test]
fn s33_an_explicit_needs_std_result_program_is_unmoved_by_the_prelude() {
    let library = fs::read_to_string(repo_root().join("std").join("result.aelys"))
        .expect("read the library result module");
    let files: Vec<(&str, &str)> = vec![
        ("root.aelys", EXPLICIT_RESULT),
        (RESULT_LIB, library.as_str()),
    ];

    let mut without = SourceOptions::default();
    without.prelude = None;
    run_row("S3.3-5a", &files, &without, 16, "1\n1\n1\n1\n");

    run_row(
        "S3.3-5b",
        &files,
        &SourceOptions::with_include(vec![repo_root()]),
        16,
        "1\n1\n1\n1\n",
    );
}

const MIXED_CARRIER: &str = r#"
needs std.result

fn main() -> i64 {
    let s: Option<i64> = result.Option::Some(40)
    let t: result.Option<i64> = Option::Some(2)
    return result.some_or(s, 0) + result.some_or(t, 0)
}
"#;

// the prelude re-exports rather than redefines, so a second `option` that cannot meet the first never exists
#[test]
fn s33_the_prelude_name_and_the_module_qualified_name_are_one_type() {
    run_row(
        "S3.3-6",
        &[("root.aelys", MIXED_CARRIER)],
        &SourceOptions::with_include(vec![repo_root()]),
        42,
        "",
    );
}

const NO_PRELUDE_NEEDED: &str = r#"
fn main() -> i64 {
    return 42
}
"#;

#[test]
fn s33_a_prelude_that_does_not_resolve_is_silent() {
    let empty = stage(&[("keep.txt", "")]);
    run_row(
        "S3.3-7a",
        &[("root.aelys", NO_PRELUDE_NEEDED)],
        &SourceOptions::default(),
        42,
        "",
    );
    run_row(
        "S3.3-7b",
        &[("root.aelys", NO_PRELUDE_NEEDED)],
        &SourceOptions::with_include(vec![empty.path().to_path_buf()]),
        42,
        "",
    );
}

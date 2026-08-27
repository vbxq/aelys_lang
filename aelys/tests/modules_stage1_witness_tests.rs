use aelys_driver::{RuntimeVariant, compile_file_with_llvm_variant, lower_file_to_air};
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

const MODULE_SEVEN: &str = "pub fn seven() -> i64 {\n    return 7\n}\n";

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

fn run_module_row(id: &str, files: Files, root: &str, exit: i32, stdout: &str) {
    for (level, opt) in LEVELS {
        let dir = stage(files);
        let root_path = dir.path().join(root);
        if let Err(err) =
            compile_file_with_llvm_variant(&root_path, *opt, false, RuntimeVariant::Rc)
        {
            panic!("{id} at {level}: the module program MUST compile and link\nerror:\n{err}");
        }
        let exe = exe_path_for(&root_path);
        assert!(
            exe.is_file(),
            "{id} at {level}: no executable was produced at {}",
            exe.display()
        );
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

fn reject_module_row(id: &str, files: Files, root: &str, present: &[&str], absent: &[&str]) {
    for (level, opt) in LEVELS {
        let dir = stage(files);
        let root_path = dir.path().join(root);
        let rendered = match lower_file_to_air(&root_path, *opt) {
            Ok(_) => panic!("{id} at {level}: MUST be rejected, but it was accepted"),
            Err(rendered) => rendered,
        };
        for needle in present {
            assert!(
                rendered.contains(needle),
                "{id} at {level}: the diagnostic MUST contain {needle:?}\nrendered:\n{rendered}"
            );
        }
        for needle in absent {
            assert!(
                !rendered.contains(needle),
                "{id} at {level}: the diagnostic MUST NOT contain {needle:?}\nrendered:\n{rendered}"
            );
        }
    }
}

fn accept_without_executable(id: &str, files: Files, root: &str) {
    for (level, opt) in LEVELS {
        let dir = stage(files);
        let root_path = dir.path().join(root);
        if let Err(err) =
            compile_file_with_llvm_variant(&root_path, *opt, false, RuntimeVariant::Rc)
        {
            panic!("{id} at {level}: MUST compile\nerror:\n{err}");
        }
        let exe = exe_path_for(&root_path);
        assert!(
            !exe.is_file(),
            "{id} at {level}: a root without `main` MUST NOT produce an executable at {}",
            exe.display()
        );
    }
}

fn golden_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/modules/base")
}

// the two lines below name the host, not the compiler, so they are the only ones neutralised
fn without_host_lines(ir: &str) -> String {
    ir.lines()
        .map(|line| {
            if line.starts_with("target datalayout = ") {
                "target datalayout = <host>"
            } else if line.starts_with("target triple = ") {
                "target triple = <host>"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn group_mod_n1_needs_free_ir_is_byte_identical() {
    let src = fs::read_to_string(golden_dir().join("base.aelys")).expect("read base.aelys");
    let golden = fs::read_to_string(golden_dir().join("base.o0.ll")).expect("read base.o0.ll");
    assert!(
        !src.contains("needs"),
        "N-1: the base program MUST be needs-free"
    );

    let dir = tempdir().expect("tempdir");
    let root_path = dir.path().join("base.aelys");
    fs::write(&root_path, &src).expect("write base.aelys");
    compile_file_with_llvm_variant(
        &root_path,
        OptimizationLevel::None,
        true,
        RuntimeVariant::Rc,
    )
    .expect("N-1: the base program MUST compile");
    let emitted = fs::read_to_string(root_path.with_extension("ll")).expect("read emitted ir");

    let emitted = without_host_lines(&emitted);
    let expected = without_host_lines(&golden);
    assert_eq!(
        emitted, expected,
        "N-1: the ir of a needs-free program MUST be byte identical to tests/modules/base/base.o0.ll"
    );
}

#[cfg(unix)]
const N3_SOURCE: &str = "fn main() -> i64 {\n    return 21\n}\n";

#[cfg(unix)]
fn n3_child_links_once() {
    let dir = tempdir().expect("tempdir");
    let root_path = dir.path().join("n3.aelys");
    fs::write(&root_path, N3_SOURCE).expect("write n3.aelys");
    compile_file_with_llvm_variant(
        &root_path,
        OptimizationLevel::None,
        false,
        RuntimeVariant::Rc,
    )
    .expect("N-3: the single-file program MUST compile and link");
    let exe = exe_path_for(&root_path);
    assert!(exe.is_file(), "N-3: no executable was produced");
    let out = Command::new(&exe).output().expect("run executable");
    assert_eq!(exit_code(&out.status), 21, "N-3: the answer MUST be 21");
}

#[cfg(unix)]
#[test]
fn group_mod_n3_empty_link_channel_adds_no_argument() {
    use std::os::unix::fs::PermissionsExt;

    if std::env::var("AELYS_MOD_N3_CHILD").is_ok() {
        n3_child_links_once();
        return;
    }

    // the parent links once with the real cc so the child never rebuilds aelys-core through it
    let warm = tempdir().expect("tempdir");
    let warm_path = warm.path().join("warm.aelys");
    fs::write(&warm_path, N3_SOURCE).expect("write warm.aelys");
    compile_file_with_llvm_variant(
        &warm_path,
        OptimizationLevel::None,
        false,
        RuntimeVariant::Rc,
    )
    .expect("N-3: the warm-up program MUST compile and link");

    let dir = tempdir().expect("tempdir");
    let bin = dir.path().join("bin");
    fs::create_dir_all(&bin).expect("create shim dir");
    let log = dir.path().join("cc.argv");
    let real_path = std::env::var("PATH").unwrap_or_default();
    let shim = bin.join("cc");
    fs::write(
        &shim,
        "#!/bin/sh\n\
         for a in \"$@\"; do printf '%s\\n' \"$a\" >> \"$AELYS_MOD_N3_LOG\"; done\n\
         printf '%s\\n' '=== end' >> \"$AELYS_MOD_N3_LOG\"\n\
         PATH=\"$AELYS_MOD_N3_REAL_PATH\" exec cc \"$@\"\n",
    )
    .expect("write shim");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).expect("chmod shim");
    fs::write(&log, "").expect("create log");

    let me = std::env::current_exe().expect("current_exe");
    let out = Command::new(&me)
        .args([
            "--exact",
            "group_mod_n3_empty_link_channel_adds_no_argument",
            "--nocapture",
            "--test-threads=1",
        ])
        .env("AELYS_MOD_N3_CHILD", "1")
        .env("AELYS_MOD_N3_LOG", &log)
        .env("AELYS_MOD_N3_REAL_PATH", &real_path)
        .env("PATH", format!("{}:{}", bin.display(), real_path))
        .output()
        .expect("spawn the linking leg");
    assert!(
        out.status.success(),
        "N-3: the linking leg MUST succeed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let recorded = fs::read_to_string(&log).expect("read shim log");
    let mut invocations: Vec<Vec<String>> = Vec::new();
    let mut current: Vec<String> = Vec::new();
    for line in recorded.lines() {
        if line == "=== end" {
            invocations.push(std::mem::take(&mut current));
        } else {
            current.push(line.to_string());
        }
    }
    let mut linkings: Vec<&Vec<String>> = invocations
        .iter()
        .filter(|args| args.iter().any(|a| a == "-laelys-core-rc"))
        .collect();
    assert_eq!(
        linkings.len(),
        1,
        "N-3: cc MUST be asked to link aelys-core exactly once, got {} of {} invocation(s):\n{recorded}",
        linkings.len(),
        invocations.len()
    );
    let args = linkings.pop().expect("one linking invocation");
    assert_eq!(
        args.len(),
        5,
        "N-3: an empty link channel MUST add no argument, cc got {args:?}"
    );
    assert_eq!(args[0], "-o", "N-3: cc argv[0] MUST be -o, got {args:?}");
    assert_eq!(
        args[2],
        format!("{}.o", args[1]),
        "N-3: cc argv[2] MUST be the object beside the executable, got {args:?}"
    );
    assert!(
        args[3].starts_with("-L") && Path::new(&args[3][2..]).is_dir(),
        "N-3: cc argv[3] MUST be the core library search dir, got {args:?}"
    );
    assert_eq!(
        args[4], "-laelys-core-rc",
        "N-3: cc argv[4] MUST be the core library, got {args:?}"
    );
}

#[test]
fn group_mod_m1_single_function_module() {
    run_module_row(
        "M-1",
        &[
            ("m.aelys", MODULE_SEVEN),
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.seven()\n}\n",
            ),
        ],
        "root.aelys",
        7,
        "",
    );
}

#[test]
fn group_mod_m9_value_crosses_the_module_boundary() {
    run_module_row(
        "M-9",
        &[
            (
                "m.aelys",
                "pub fn hi() -> i64 {\n    println(\"hi\")\n    return 0\n}\n",
            ),
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return m.hi()\n}\n",
            ),
        ],
        "root.aelys",
        0,
        "hi\n",
    );
}

#[test]
fn group_mod_q1i_plain_path_import_calls_across() {
    run_module_row(
        "Q-1(i)",
        &[
            ("a/b.aelys", MODULE_SEVEN),
            (
                "root.aelys",
                "needs a.b\n\nfn main() -> i64 {\n    return b.seven()\n}\n",
            ),
        ],
        "root.aelys",
        7,
        "",
    );
}

#[test]
fn group_mod_q1ii_aliased_path_import_calls_across() {
    run_module_row(
        "Q-1(ii)",
        &[
            ("a/b.aelys", MODULE_SEVEN),
            (
                "root.aelys",
                "needs a.b as k\n\nfn main() -> i64 {\n    return k.seven()\n}\n",
            ),
        ],
        "root.aelys",
        7,
        "",
    );
}

#[test]
fn group_mod_q1iii_single_symbol_import_calls_across() {
    run_module_row(
        "Q-1(iii)",
        &[
            ("a/b.aelys", MODULE_SEVEN),
            (
                "root.aelys",
                "needs seven from a.b\n\nfn main() -> i64 {\n    return seven()\n}\n",
            ),
        ],
        "root.aelys",
        7,
        "",
    );
}

#[test]
fn group_mod_q2_intra_module_call_keeps_its_module() {
    run_module_row(
        "Q-2",
        &[
            (
                "a/b.aelys",
                "fn helper() -> i64 {\n    return 7\n}\n\npub fn tripled() -> i64 {\n    return helper() * 3\n}\n",
            ),
            (
                "root.aelys",
                "needs a.b\n\nfn main() -> i64 {\n    return b.tripled()\n}\n",
            ),
        ],
        "root.aelys",
        21,
        "",
    );
}

#[test]
fn group_mod_q3_global_function_reference_crosses() {
    run_module_row(
        "Q-3",
        &[
            (
                "a/b.aelys",
                "pub fn seven() -> i64 {\n    return 7\n}\n\npub let f: fn() -> i64 = seven\n",
            ),
            (
                "root.aelys",
                "needs a.b\n\nfn main() -> i64 {\n    return b.f()\n}\n",
            ),
        ],
        "root.aelys",
        7,
        "",
    );
}

const TYPE_ERROR_MODULE: &str = "pub fn bad() -> i64 {\n    return \"x\"\n}\n";

#[test]
fn group_mod_t4_imported_module_is_compiled_and_named() {
    reject_module_row(
        "T-4",
        &[("m.aelys", TYPE_ERROR_MODULE), ("root.aelys", "needs m\n")],
        "root.aelys",
        &["m.aelys", "return \"x\"", "[E0301]"],
        &[],
    );
}

#[test]
fn group_mod_t4b_valid_import_only_root_produces_no_executable() {
    accept_without_executable(
        "T-4b",
        &[("m.aelys", MODULE_SEVEN), ("root.aelys", "needs m\n")],
        "root.aelys",
    );
}

#[test]
fn group_mod_x1_module_borrow_error_names_its_own_file() {
    reject_module_row(
        "X-1",
        &[
            (
                "m.aelys",
                "struct Resource { id: i64 }\n\npub fn leak() -> i64 {\n    let a = Resource { id: 1 }\n    let b = a\n    return a.id\n}\n",
            ),
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return 0\n}\n",
            ),
        ],
        "root.aelys",
        &["m.aelys", "return a.id", "[E0701]"],
        &[],
    );
}

#[test]
fn group_mod_x2_topological_order_reports_the_dependency_first() {
    reject_module_row(
        "X-2",
        &[
            ("m.aelys", TYPE_ERROR_MODULE),
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return \"y\"\n}\n",
            ),
        ],
        "root.aelys",
        &["m.aelys", "return \"x\""],
        &["root.aelys", "return \"y\""],
    );
}

#[test]
fn group_mod_x2b_discovery_order_reports_the_root_first() {
    reject_module_row(
        "X-2b",
        &[
            ("m.aelys", TYPE_ERROR_MODULE),
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return 1 +\n}\n",
            ),
        ],
        "root.aelys",
        &["root.aelys", "[E0102]"],
        &["m.aelys", "return \"x\""],
    );
}

#[test]
fn group_mod_t1_empty_module_imported_and_unused() {
    run_module_row(
        "T-1",
        &[
            ("m.aelys", ""),
            (
                "root.aelys",
                "needs m\n\nfn main() -> i64 {\n    return 0\n}\n",
            ),
        ],
        "root.aelys",
        0,
        "",
    );
}

use aelys_driver::{compile_file_with_llvm_variant, RuntimeVariant};
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn exe_path_for(p: &Path) -> PathBuf {
    let mut o = p.with_extension("");
    if cfg!(windows) {
        o.set_extension("exe");
    }
    o
}

fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found") || error.contains("failed to run")
}

fn run_with_stats(src: &str, variant: RuntimeVariant) -> Option<(i32, String)> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm_variant(&source_path, OptimizationLevel::None, false, variant) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                eprintln!("linker unavailable; skipping exec assertion");
                return None;
            }
            panic!("compilation/link should succeed: {err}");
        }
    }

    let exe = exe_path_for(&source_path);
    if !exe.is_file() {
        eprintln!("executable not produced (linker unavailable); skipping");
        return None;
    }

    let output = Command::new(&exe)
        .env("AELYS_RC_STATS", "1")
        .output()
        .expect("run compiled exe");
    let code = output.status.code().expect("exit code");
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    Some((code, stderr))
}

#[test]
fn s1_unshared_rc_allocs_one_frees_one() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn main() -> i64 {
    let r: Rc<i64> = Rc::new(7)
    return Rc::get(r)
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(code, 7, "expected the value read through the handle; stderr:\n{stderr}");
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "expected balanced alloc/free for one unshared Rc; got stderr:\n{stderr}"
    );
}

#[test]
fn s2_shared_rc_frees_once_not_per_handle() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let b: Rc<i64> = a
    return Rc::get(b)
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(code, 7, "expected value through the shared handle; stderr:\n{stderr}");
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "shared Rc must free once at the last drop, not per handle; got stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("frees=2"),
        "a shared Rc must NOT free twice; got stderr:\n{stderr}"
    );
}

#[test]
fn s3_stage1_shapes_run_under_real_rc_without_crash() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn touch(a: Rc<i64>) -> i64 { return 1 }
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let b: Rc<i64> = a
    let c: Rc<i64> = b
    let n: i64 = touch(c)
    return n + 41
}
"#,
        RuntimeVariant::Rc,
    ) else {
        return;
    };
    assert_eq!(code, 42, "expected deterministic exit 42 under real rc; stderr:\n{stderr}");
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "balanced insertion: one acyclic object allocated and freed once; got:\n{stderr}"
    );
}

#[test]
fn s4_leak_variant_never_frees() {
    let Some((code, stderr)) = run_with_stats(
        r#"
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let b: Rc<i64> = a
    return Rc::get(b)
}
"#,
        RuntimeVariant::Leak,
    ) else {
        return;
    };
    assert_eq!(code, 7, "leak variant must still compute the value; stderr:\n{stderr}");
    assert!(
        stderr.contains("[rc] allocs=1 frees=0"),
        "the leak variant must never free (frees=0); got stderr:\n{stderr}"
    );
}

fn find_core_archive(file: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, file: &str) -> Option<PathBuf> {
        let entries = fs::read_dir(dir).ok()?;
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if let Some(found) = walk(&path, file) {
                    return Some(found);
                }
            } else if path.file_name().and_then(|s| s.to_str()) == Some(file) {
                return Some(path);
            }
        }
        None
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let root = manifest.parent().unwrap_or(manifest);
    walk(&root.join("target"), file)
}

#[test]
fn s5_rc_symbols_present_in_both_renamed_archives() {
    for variant in ["leak", "rc"] {
        let file = format!("libaelys-core-{variant}.a");
        let Some(archive) = find_core_archive(&file) else {
            panic!("expected {file} to exist (build the workspace first)");
        };
        let Ok(out) = Command::new("nm").arg(&archive).output() else {
            eprintln!("nm unavailable; skipping symbol assertion");
            return;
        };
        let syms = String::from_utf8_lossy(&out.stdout);
        for sym in [
            "__aelys_rc_retain",
            "__aelys_rc_release",
            "__aelys_arc_retain",
            "__aelys_arc_release",
        ] {
            assert!(
                syms.contains(sym),
                "{archive:?} must export {sym}; nm output:\n{syms}"
            );
        }
    }
}

#[test]
fn asan_shared_rc_is_clean() {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(
        &source_path,
        r#"
fn main() -> i64 {
    let a: Rc<i64> = Rc::new(7)
    let b: Rc<i64> = a
    return Rc::get(b)
}
"#,
    )
    .expect("write source");

    match compile_file_with_llvm_variant(&source_path, OptimizationLevel::None, false, RuntimeVariant::Rc) {
        Ok(()) => {}
        Err(err) => {
            if linker_unavailable(&err.to_string()) {
                eprintln!("linker unavailable; skipping ASan probe");
                return;
            }
            panic!("compilation should succeed: {err}");
        }
    }

    let object = source_path.with_extension(if cfg!(windows) { "obj" } else { "o" });
    if !object.is_file() {
        eprintln!("object not produced; skipping ASan probe");
        return;
    }

    let Some(archive) = find_core_archive("libaelys-core-rc.a") else {
        eprintln!("rc archive not found; skipping ASan probe");
        return;
    };
    let lib_dir = archive.parent().expect("archive has a parent");

    let asan_exe = dir.path().join("module_asan");
    let link = Command::new("clang")
        .arg("-fsanitize=address")
        .arg("-g")
        .arg(&object)
        .arg(format!("-L{}", lib_dir.display()))
        .arg("-laelys-core-rc")
        .arg("-o")
        .arg(&asan_exe)
        .output();
    let link = match link {
        Ok(out) => out,
        Err(_) => {
            eprintln!("clang unavailable; skipping ASan probe");
            return;
        }
    };
    assert!(
        link.status.success(),
        "ASan link failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&link.stdout),
        String::from_utf8_lossy(&link.stderr)
    );

    let run = Command::new(&asan_exe)
        .env("AELYS_RC_STATS", "1")
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .output()
        .expect("run ASan-instrumented exe");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(
        run.status.code(),
        Some(7),
        "ASan run must exit 7 (no sanitizer abort); stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "ASan must report no errors (no double-free/UAF/leak); stderr:\n{stderr}"
    );
    assert!(
        stderr.contains("[rc] allocs=1 frees=1"),
        "shared Rc under ASan must still free exactly once; stderr:\n{stderr}"
    );
}

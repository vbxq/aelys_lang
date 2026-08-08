// no borrow checking this stage; these assert the exact process exit code.

use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::tempdir;

fn linker_unavailable(error: &str) -> bool {
    error.contains("program not found")
        || error.contains("failed to run")
        || error.contains("failed with status Some(-1073741819)")
}

fn exe_path_for(source_path: &Path) -> PathBuf {
    let mut output = source_path.with_extension("");
    if cfg!(windows) {
        output.set_extension("exe");
    }
    output
}

/// compile `src`, link, run, and return the exit code. `none` only when the
fn compile_and_run(src: &str) -> Option<i32> {
    compile_and_run_opt(src, OptimizationLevel::None)
}

fn compile_and_run_opt(src: &str, level: OptimizationLevel) -> Option<i32> {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, src).expect("write source");

    match compile_file_with_llvm(&source_path, level, false) {
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

    let output = Command::new(&exe).output().expect("run compiled exe");
    Some(output.status.code().expect("exit code"))
}

fn find_core_archive(file: &str) -> Option<PathBuf> {
    fn walk(dir: &Path, file: &str) -> Option<PathBuf> {
        for entry in fs::read_dir(dir).ok()?.flatten() {
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
fn p1_ref_read_through_shared() {
    let Some(code) = compile_and_run(
        r#"
fn read(r: &i64) -> i64 { return *r }
fn main() -> i64 { let x = 41; return read(&x) + 1 }
"#,
    ) else {
        return;
    };
    assert_eq!(code, 42, "P1: read(&x)+1 must be 42");
}

#[test]
fn p2_ref_write_through_mut() {
    let Some(code) = compile_and_run(
        r#"
fn bump(r: &mut i64) { *r = *r + 1 }
fn main() -> i64 { let mut x = 41; bump(&mut x); return x }
"#,
    ) else {
        return;
    };
    assert_eq!(code, 42, "P2: bump(&mut x) must leave x = 42");
}

// slice an array and index-read it (fat pointer + index).
#[test]
fn p3_slice_array_index_read() {
    let Some(code) = compile_and_run(
        r#"
fn main() -> i64 { let a = [10, 20, 12]; let s = a[..]; return s[0] + s[2] }
"#,
    ) else {
        return;
    };
    assert_eq!(code, 22, "P3: s[0] + s[2] must be 22");
}

// p4: pass a slice &[t] to a function (slice param abi).
#[test]
fn p4_pass_slice_to_fn() {
    let Some(code) = compile_and_run(
        r#"
fn sum3(s: &[i64]) -> i64 { return s[0] + s[1] + s[2] }
fn main() -> i64 { let a = [40, 1, 1]; return sum3(a[..]) }
"#,
    ) else {
        return;
    };
    assert_eq!(code, 42, "P4: sum3(a[..]) must be 42");
}

#[test]
fn p5_write_through_mut_slice() {
    let Some(code) = compile_and_run(
        r#"
fn set0(s: &mut [i64]) { s[0] = 42 }
fn main() -> i64 { let mut a = [0, 7, 7]; set0(a[..]); return a[0] }
"#,
    ) else {
        return;
    };
    assert_eq!(code, 42, "P5: set0(a[..]) must leave a[0] = 42");
}

const P6_SRC: &str = r#"
fn fill(r: &mut i64, s: &[i64]) { *r = s[0] + s[1] }
fn main() -> i64 {
    let buf = [7, 8, 9]
    let mut out = 0
    fill(&mut out, buf[..])
    return out
}
"#;

#[test]
fn p6_asan_seam_exit_15() {
    let Some(code) = compile_and_run(P6_SRC) else {
        return;
    };
    assert_eq!(code, 15, "P6: fill(&mut out, buf[..]) must leave out = 15");
}

// p6 (asan): best-effort probe that the same program is memory-clean (no uaf/oob),
// since both the &mut write and the slice read point into stack allocas.
#[test]
fn p6_asan_clean_probe() {
    let dir = tempdir().expect("tempdir");
    let source_path = dir.path().join("module.aelys");
    fs::write(&source_path, P6_SRC).expect("write source");

    match compile_file_with_llvm(&source_path, OptimizationLevel::None, false) {
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
    let Some(archive) = find_core_archive("libaelys-core.a") else {
        eprintln!("core archive not found; skipping ASan probe");
        return;
    };
    let lib_dir = archive.parent().expect("archive has a parent");

    let asan_exe = dir.path().join("module_asan");
    let link = Command::new("clang")
        .arg("-fsanitize=address")
        .arg("-g")
        .arg(&object)
        .arg(format!("-L{}", lib_dir.display()))
        .arg("-laelys-core")
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
    if !link.status.success() {
        eprintln!(
            "ASan link failed (skipping probe):\nstderr:\n{}",
            String::from_utf8_lossy(&link.stderr)
        );
        return;
    }

    let run = Command::new(&asan_exe)
        .env("ASAN_OPTIONS", "detect_leaks=1")
        .output()
        .expect("run ASan-instrumented exe");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert_eq!(
        run.status.code(),
        Some(15),
        "ASan run must exit 15 (no sanitizer abort); stderr:\n{stderr}"
    );
    assert!(
        !stderr.contains("AddressSanitizer") && !stderr.contains("LeakSanitizer"),
        "ASan must report no errors (no UAF/OOB/leak); stderr:\n{stderr}"
    );
}

// regression: a &mut borrow of a mutable local in a loop must invalidate local
// const-prop, else the optimizer folds the stale value (82 vs the true 141) at o2/o3.
// the borrow target is `let mut x`, since `&mut` of an immutable binding
const LOOP_BORROW_SRC: &str = r#"
fn main() -> i64 {
    let mut x = 41
    let mut i = 0
    let mut acc = 0
    while i < 2 {
        acc = acc + x
        let r = &mut x
        *r = 100
        i = i + 1
    }
    return acc
}
"#;

#[test]
fn p7_const_prop_loop_borrow_o0() {
    let Some(code) = compile_and_run_opt(LOOP_BORROW_SRC, OptimizationLevel::None) else {
        return;
    };
    assert_eq!(code, 141, "P7 O0: loop &mut borrow, acc must be 141");
}

#[test]
fn p7_const_prop_loop_borrow_o2_standard() {
    let Some(code) = compile_and_run_opt(LOOP_BORROW_SRC, OptimizationLevel::Standard) else {
        return;
    };
    assert_eq!(
        code, 141,
        "P7 O2: const-prop must not fold x across a &mut borrow in a loop"
    );
}

#[test]
fn p7_const_prop_loop_borrow_o3_aggressive() {
    let Some(code) = compile_and_run_opt(LOOP_BORROW_SRC, OptimizationLevel::Aggressive) else {
        return;
    };
    assert_eq!(
        code, 141,
        "P7 O3: const-prop must not fold x across a &mut borrow in a loop"
    );
}

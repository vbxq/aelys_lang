use aelys_driver::run_file;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

fn build_fixture(dir_name: &str, package_name: &str) -> PathBuf {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crate_dir = manifest_dir.join("tests/fixtures").join(dir_name);
    let target_dir = manifest_dir.join("target").join(dir_name);

    let status = Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(crate_dir.join("Cargo.toml"))
        .arg("--release")
        .env("CARGO_TARGET_DIR", &target_dir)
        .status()
        .expect("cargo build should run");
    assert!(status.success(), "fixture build failed: {}", dir_name);

    let lib_name = lib_filename(package_name);
    let lib_path = target_dir.join("release").join(lib_name);
    assert!(lib_path.exists(), "missing built library at {:?}", lib_path);
    lib_path
}

fn lib_filename(package_name: &str) -> String {
    let base = package_name.replace('-', "_");
    if cfg!(target_os = "linux") {
        format!("lib{}.so", base)
    } else if cfg!(target_os = "macos") {
        format!("lib{}.dylib", base)
    } else if cfg!(target_os = "windows") {
        format!("{}.dll", base)
    } else {
        panic!("unsupported target for native dependency tests");
    }
}

fn module_ext() -> &'static str {
    if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    }
}

fn create_module_env() -> TempDir {
    tempfile::tempdir().expect("Failed to create temp dir")
}

fn write_file(dir: &TempDir, path: &str, content: &str) -> PathBuf {
    let file_path = dir.path().join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("Failed to create parent directories");
    }
    fs::write(&file_path, content).expect("Failed to write file");
    file_path
}

fn copy_module(dir: &TempDir, module_name: &str, lib_path: &Path) {
    let dest = dir.path().join(format!("{}.{}", module_name, module_ext()));
    fs::copy(lib_path, dest).expect("copy native module");
}

#[test]
fn circular_native_dependency_is_rejected() {
    let lib_a = build_fixture("native_cycle_a", "aelys-native-cycle-a");
    let lib_b = build_fixture("native_cycle_b", "aelys-native-cycle-b");

    let dir = create_module_env();
    copy_module(&dir, "cycle_a", &lib_a);
    copy_module(&dir, "cycle_b", &lib_b);

    let main_path = write_file(&dir, "main.aelys", "needs cycle_a\n");

    let err = run_file(&main_path).expect_err("should fail");
    assert!(
        err.to_string().contains("circular dependency"),
        "unexpected error: {}",
        err
    );
}

#[test]
fn diamond_dependency_version_conflict_is_rejected() {
    let lib_a = build_fixture("native_dep_a", "aelys-native-dep-a");
    let lib_b = build_fixture("native_dep_b", "aelys-native-dep-b");
    let lib_c = build_fixture("native_dep_c", "aelys-native-dep-c");

    let dir = create_module_env();
    copy_module(&dir, "dep_a", &lib_a);
    copy_module(&dir, "dep_b", &lib_b);
    copy_module(&dir, "dep_c", &lib_c);

    let main_path = write_file(&dir, "main.aelys", "needs dep_a\nneeds dep_c\n");

    let err = run_file(&main_path).expect_err("should fail");
    assert!(
        err.to_string().contains("version conflict"),
        "unexpected error: {}",
        err
    );
}

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
        panic!("unsupported target for native tamper tests");
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

#[test]
fn invalid_exports_hash_is_rejected() {
    let lib = build_fixture("native_tamper", "aelys-native-tamper");
    let dir = create_module_env();

    let dest = dir.path().join(format!("tamper.{}", module_ext()));
    fs::copy(&lib, &dest).expect("copy native module");

    let main_path = write_file(&dir, "main.aelys", "needs tamper\n");

    let err = run_file(&main_path).expect_err("should fail");
    assert!(
        err.to_string().contains("exports_hash"),
        "unexpected error: {}",
        err
    );
}

#[test]
fn zero_exports_hash_is_rejected() {
    let lib = build_fixture("native_zero_hash", "aelys-native-zero-hash");
    let dir = create_module_env();

    let dest = dir.path().join(format!("zero_hash.{}", module_ext()));
    fs::copy(&lib, &dest).expect("copy native module");

    let main_path = write_file(&dir, "main.aelys", "needs zero_hash\n");

    let err = run_file(&main_path).expect_err("should fail");
    assert!(
        err.to_string().contains("exports_hash"),
        "unexpected error: {}",
        err
    );
}

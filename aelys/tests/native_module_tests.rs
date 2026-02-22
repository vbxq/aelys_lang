use aelys_driver::run_file;
use std::fs::{self, File};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};
use tempfile::TempDir;

fn native_build_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn build_test_cdylib() -> PathBuf {
    let _guard = native_build_lock()
        .lock()
        .expect("native build lock should not be poisoned");
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let crate_dir = manifest_dir.join("tests/fixtures/native_test");
    let target_dir = manifest_dir.join("target/native_test");
    let lib_name = if cfg!(target_os = "linux") {
        "libaelys_native_test.so"
    } else if cfg!(target_os = "macos") {
        "libaelys_native_test.dylib"
    } else if cfg!(target_os = "windows") {
        "aelys_native_test.dll"
    } else {
        panic!("unsupported target for native module test");
    };
    let lib_path = target_dir.join("release").join(lib_name);
    if lib_path.exists() {
        return lib_path;
    }

    let status = Command::new("cargo")
        .arg("build")
        .arg("--manifest-path")
        .arg(crate_dir.join("Cargo.toml"))
        .arg("--release")
        .env("CARGO_TARGET_DIR", &target_dir)
        .status()
        .expect("cargo build should run");
    assert!(status.success(), "native test crate build failed");
    assert!(lib_path.exists(), "missing built library at {:?}", lib_path);
    lib_path
}

fn create_module_env() -> TempDir {
    tempfile::tempdir().expect("Failed to create temp dir")
}

fn write_file(dir: &TempDir, path: &str, content: &str) -> PathBuf {
    let file_path = dir.path().join(path);
    if let Some(parent) = file_path.parent() {
        fs::create_dir_all(parent).expect("Failed to create parent directories");
    }
    let mut file = File::create(&file_path).expect("Failed to create file");
    write!(file, "{}", content).expect("Failed to write file");
    file_path
}

#[test]
fn script_imports_native_module() {
    let dir = create_module_env();
    let lib_path = build_test_cdylib();

    let ext = if cfg!(target_os = "windows") {
        "dll"
    } else if cfg!(target_os = "macos") {
        "dylib"
    } else {
        "so"
    };

    let module_lib_path = dir.path().join(format!("native_test.{}", ext));
    fs::copy(&lib_path, &module_lib_path).expect("copy native module");

    let main_path = write_file(
        &dir,
        "main.aelys",
        r#"
needs native_test
native_test.add(5, 5)
"#,
    );

    let result = run_file(&main_path).expect("native module import should succeed");
    assert_eq!(result.as_int(), Some(10));
}

use aelys_modules::native::NativeLoader;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::{Mutex, OnceLock};

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
        panic!("unsupported target for native loader test");
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

#[test]
fn load_native_descriptor_and_exports() {
    let loader = NativeLoader::new();
    let lib_path = build_test_cdylib();
    let module = loader
        .load_dynamic("native_test", &lib_path)
        .expect("load native module");
    assert!(module.exports.contains_key("add"));
}

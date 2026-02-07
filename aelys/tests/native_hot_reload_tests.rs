use aelys_driver::modules::ModuleLoader;
use aelys_runtime::{VM, VmConfig};
use aelys_syntax::Source;
use aelys_syntax::Span;
use aelys_syntax::{ImportKind, NeedsStmt};
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
        panic!("unsupported target for native hot reload tests");
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
fn hot_reload_requires_dev_flag() {
    let lib_a = build_fixture("native_hot_a", "aelys-native-hot-a");
    let lib_b = build_fixture("native_hot_b", "aelys-native-hot-b");

    let dir = create_module_env();
    let module_path = dir.path().join(format!("hot_mod.{}", module_ext()));
    fs::copy(&lib_a, &module_path).expect("copy hot module A");

    let entry_path = write_file(&dir, "main.aelys", "needs hot_mod\n");
    let source = Source::new(entry_path.display().to_string(), "needs hot_mod\n");

    let config = VmConfig::default();
    let mut vm =
        VM::with_config_and_args(source.clone(), config, Vec::new()).expect("vm should initialize");
    let mut loader = ModuleLoader::new(&entry_path, source);

    let needs = NeedsStmt {
        path: vec!["hot_mod".to_string()],
        kind: ImportKind::Module { alias: None },
        span: Span::dummy(),
    };

    loader
        .load_module(&needs, &mut vm)
        .expect("initial native module load should succeed");

    replace_module_binary(&module_path, &lib_b);

    let err = loader
        .load_module(&needs, &mut vm)
        .err()
        .expect("should fail when hot reload is disabled");
    assert!(
        err.to_string().contains("hot reload disabled"),
        "unexpected error: {}",
        err
    );
}

fn replace_module_binary(dest: &Path, new_lib: &Path) {
    #[cfg(target_os = "windows")]
    {
        let parent = dest.parent().expect("module path should have parent");
        let stem = dest
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("module");
        let ext = dest.extension().and_then(|s| s.to_str()).unwrap_or("dll");
        let unique = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        let instance = parent.join(format!("{}.{}.{}", stem, unique, ext));
        fs::copy(new_lib, &instance).expect("write hot module instance");
    }
    #[cfg(not(target_os = "windows"))]
    {
        let temp_path = dest.with_extension(format!("{}.tmp", module_ext()));
        fs::copy(new_lib, &temp_path).expect("stage hot module replacement");
        fs::rename(&temp_path, dest).expect("replace hot module with new binary");
    }
}

use aelys_driver::compile_file_with_llvm;
use aelys_opt::OptimizationLevel;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn write_temp_source(prefix: &str, source: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough for test naming")
        .as_nanos();
    path.push(format!("aelys_driver_{prefix}_{stamp}.aelys"));
    fs::write(&path, source).expect("failed to write temp source file");
    path
}

#[test]
fn emit_llvm_ir_mode_skips_object_and_link_steps() {
    let path = write_temp_source(
        "emit_ir_only",
        r#"
fn main() -> i64 {
    return 42
}
"#,
    );

    compile_file_with_llvm(&path, OptimizationLevel::None, true)
        .expect("emit-llvm-ir mode should not require object emission or linking");

    let ll_path = path.with_extension("ll");
    assert!(ll_path.exists(), "llvm ir file should be generated");
    assert!(
        !path.with_extension("obj").exists() && !path.with_extension("o").exists(),
        "emit-llvm-ir mode should not emit an object file"
    );
    assert!(
        !path.with_extension("exe").exists(),
        "emit-llvm-ir mode should not invoke the native linker"
    );

    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(&ll_path);
}

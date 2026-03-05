use aelys_common::{WarningKind, format_warnings};
use aelys_driver::compile_file_with_llvm_with_warnings;
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
fn compile_pipeline_returns_structured_warnings() {
    let path = write_temp_source(
        "warning_diag",
        r#"
struct Point { x: i64 }
struct Point { y: i64 }

fn probe() -> i64 {
    return 0
}
"#,
    );

    let warnings =
        compile_file_with_llvm_with_warnings(&path, OptimizationLevel::None, false)
            .expect("program should compile and only emit a warning");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("obj"));
    let _ = fs::remove_file(path.with_extension("o"));

    assert!(
        warnings
            .iter()
            .any(|warning| matches!(warning.kind, WarningKind::UnknownType { .. })),
        "expected at least one warning from duplicate struct declarations"
    );
    assert!(
        warnings.iter().all(|warning| warning.source.is_some()),
        "all warnings should carry source information for snippet rendering"
    );

    let rendered = format_warnings(&warnings);
    assert!(rendered.contains("warning[W"), "{rendered}");
    assert!(rendered.contains(" --> "), "{rendered}");
}

use aelys_common::{WarningKind, format_warnings};
use aelys_driver::{RuntimeVariant, compile_file_with_llvm_with_warnings};
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
@inline
fn probe(n: i64) -> i64 {
    if n <= 0 {
        return 0
    }
    return probe(n - 1)
}

fn main() -> i64 {
    return probe(3)
}
"#,
    );

    let warnings = compile_file_with_llvm_with_warnings(
        &path,
        OptimizationLevel::Basic,
        false,
        RuntimeVariant::default(),
    )
    .expect("program should compile and only emit a warning");
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("obj"));
    let _ = fs::remove_file(path.with_extension("o"));

    assert!(
        warnings
            .iter()
            .any(|warning| matches!(warning.kind, WarningKind::InlineRecursive)),
        "expected the inliner's refusal to inline a recursive `@inline` function"
    );
    assert!(
        warnings.iter().all(|warning| warning.source.is_some()),
        "all warnings should carry source information for snippet rendering"
    );

    let rendered = format_warnings(&warnings);
    assert!(rendered.contains("warning[W0101]"), "{rendered}");
    assert!(rendered.contains(" --> "), "{rendered}");
}

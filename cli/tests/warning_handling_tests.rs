use aelys_cli::cli::run_with_args;
use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

fn write_temp_source(prefix: &str, source: &str) -> PathBuf {
    let mut path = std::env::temp_dir();
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("clock should be monotonic enough for test naming")
        .as_nanos();
    path.push(format!("aelys_cli_{prefix}_{stamp}.aelys"));
    fs::write(&path, source).expect("failed to write temp source file");
    path
}

#[test]
fn werror_turns_warnings_into_failure() {
    let path = write_temp_source(
        "werror",
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

    let args = vec![
        "aelys".to_string(),
        "compile".to_string(),
        path.display().to_string(),
        "-Werror".to_string(),
    ];

    let result = run_with_args(&args);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("obj"));
    let _ = fs::remove_file(path.with_extension("o"));

    let err = result.expect_err("expected -Werror to fail on warning");
    assert!(err.contains("aborting due to"), "{err}");
}

#[test]
fn disabling_inline_warnings_allows_build() {
    let path = write_temp_source(
        "wdisable",
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

    let args = vec![
        "aelys".to_string(),
        "compile".to_string(),
        path.display().to_string(),
        "-Werror".to_string(),
        "-Wno-inline".to_string(),
    ];

    let result = run_with_args(&args);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("obj"));
    let _ = fs::remove_file(path.with_extension("o"));

    assert!(
        result.is_ok(),
        "inline warnings were disabled, got {result:?}"
    );
}

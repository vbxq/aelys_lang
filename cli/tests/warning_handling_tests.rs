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
struct Point { x: i64 }
struct Point { y: i64 }

fn probe() -> i64 {
    return 0
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
fn disabling_type_warnings_allows_build() {
    let path = write_temp_source(
        "wdisable",
        r#"
struct Point { x: i64 }
struct Point { y: i64 }

fn probe() -> i64 {
    return 0
}
"#,
    );

    let args = vec![
        "aelys".to_string(),
        "compile".to_string(),
        path.display().to_string(),
        "-Werror".to_string(),
        "-Wno-type".to_string(),
    ];

    let result = run_with_args(&args);
    let _ = fs::remove_file(&path);
    let _ = fs::remove_file(path.with_extension("obj"));
    let _ = fs::remove_file(path.with_extension("o"));

    assert!(
        result.is_ok(),
        "type warnings were disabled, got {result:?}"
    );
}

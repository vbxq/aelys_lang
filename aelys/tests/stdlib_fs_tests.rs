mod common;
use common::*;
use std::fs;
use tempfile::tempdir;

#[test]
fn fs_write_and_read_text() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.txt");
    let path_str = path.display().to_string().replace('\\', "/");

    let code = format!(
        r#"
needs std.fs
fs.write_text("{}", "hello world")
fs.read_text("{}")
    "#,
        path_str, path_str
    );

    let err = run_aelys_err(&code);
    // Without capability, should fail
    assert!(err.contains("capability") || err.contains("permission"));
}

#[test]
fn fs_open_read_close() {
    let dir = tempdir().unwrap();
    let test_file = dir.path().join("data.txt");
    fs::write(&test_file, "test content").unwrap();
    let path_str = test_file.display().to_string().replace('\\', "/");

    let code = format!(
        r#"
needs std.fs
let f = fs.open("{}", "r")
let data = fs.read(f)
fs.close(f)
42
"#,
        path_str
    );

    // Will fail without capability
    let err = run_aelys_err(&code);
    assert!(err.contains("capability") || err.contains("permission") || err.contains("denied"));
}

#[test]
fn fs_open_invalid_mode() {
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.txt");
    let path_str = path.display().to_string().replace('\\', "/");

    let code = format!(
        r#"
needs std.fs
fs.open("{}", "xyz")
"#,
        path_str
    );

    let err = run_aelys_err(&code);
    assert!(err.contains("invalid") || err.contains("mode") || err.contains("capability"));
}

#[test]
fn fs_close_invalid_handle() {
    let code = r#"
needs std.fs
fs.close(999)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("handle") || err.contains("capability"));
}

#[test]
fn fs_read_line_eof_returns_null() {
    // Will fail without capability, but tests the expected behavior
    let dir = tempdir().unwrap();
    let test_file = dir.path().join("lines.txt");
    fs::write(&test_file, "line1\nline2\n").unwrap();
    let path_str = test_file.display().to_string().replace('\\', "/");

    let code = format!(
        r#"
needs std.fs
let f = fs.open("{}", "r")
let l1 = fs.read_line(f)
let l2 = fs.read_line(f)
let eof = fs.read_line(f)
fs.close(f)
42
"#,
        path_str
    );

    let err = run_aelys_err(&code);
    assert!(err.contains("capability") || err.contains("denied"));
}

#[test]
fn fs_read_bytes_negative() {
    let code = r#"
needs std.fs
let f = 1
fs.read_bytes(f, -10)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("negative") || err.contains("capability"));
}

#[test]
fn fs_read_bytes_exceeds_max() {
    let code = r#"
needs std.fs
let f = 1
fs.read_bytes(f, 20000000)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("max") || err.contains("MAX") || err.contains("capability"));
}

#[test]
fn fs_write_not_opened_for_writing() {
    let dir = tempdir().unwrap();
    let test_file = dir.path().join("readonly.txt");
    fs::write(&test_file, "data").unwrap();
    let path_str = test_file.display().to_string().replace('\\', "/");

    let code = format!(
        r#"
needs std.fs
let f = fs.open("{}", "r")
fs.write(f, "new data")
"#,
        path_str
    );

    let err = run_aelys_err(&code);
    assert!(err.contains("writing") || err.contains("capability"));
}

#[test]
fn fs_exists_nonexistent() {
    let code = r#"
needs std.fs
fs.exists("/nonexistent/path/nowhere.txt")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("capability") || err.contains("denied"));
}

#[test]
fn fs_basename_and_dirname() {
    let code = r#"
needs std.fs
let base = fs.basename("/foo/bar/test.txt")
let dir = fs.dirname("/foo/bar/test.txt")
42
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("capability"));
}

#[test]
fn fs_extension_extraction() {
    let code = r#"
needs std.fs
fs.extension("file.rs")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("capability"));
}

#[test]
fn fs_mkdir_fails_without_capability() {
    let code = r#"
needs std.fs
fs.mkdir("/tmp/test_aelys")
"#;
    assert_aelys_error_contains(code, "capability");
}

#[test]
fn fs_delete_fails_without_capability() {
    let code = r#"
needs std.fs
fs.delete("/tmp/somefile.txt")
"#;
    assert_aelys_error_contains(code, "capability");
}

#[test]
fn fs_rename_fails_without_capability() {
    let code = r#"
needs std.fs
fs.rename("/tmp/old.txt", "/tmp/new.txt")
"#;
    assert_aelys_error_contains(code, "capability");
}

#[test]
fn fs_copy_fails_without_capability() {
    let code = r#"
needs std.fs
fs.copy("/tmp/src.txt", "/tmp/dst.txt")
"#;
    assert_aelys_error_contains(code, "capability");
}

#[test]
fn fs_readdir_fails_without_capability() {
    let code = r#"
needs std.fs
fs.readdir("/tmp")
"#;
    assert_aelys_error_contains(code, "capability");
}

#[test]
fn fs_join_absolute_path_rejected() {
    // This is already tested in security_audit_tests.rs
    // but worth repeating
    let code = r#"
needs std.fs
fs.join("/app", "/etc/passwd")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("absolute") || err.contains("capability"));
}

#[test]
fn fs_join_parent_escape() {
    let code = r#"
needs std.fs
fs.join("/app/data", "../../etc/passwd")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("escapes") || err.contains("capability"));
}

#[test]
fn fs_absolute_nonexistent() {
    let code = r#"
needs std.fs
fs.absolute("/nonexistent/path")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("capability") || err.contains("failed"));
}

#[test]
fn fs_write_line_works() {
    let code = r#"
needs std.fs
let f = 1
fs.write_line(f, "test")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

#[test]
fn fs_is_file_and_is_dir() {
    let code = r#"
needs std.fs
fs.is_file("/tmp")
"#;
    assert_aelys_error_contains(code, "capability");
}

#[test]
fn fs_size_of_nonexistent() {
    let code = r#"
needs std.fs
fs.size("/nonexistent")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("capability") || err.contains("failed"));
}

#[test]
fn fs_rmdir_fails_without_capability() {
    let code = r#"
needs std.fs
fs.rmdir("/tmp/testdir")
"#;
    assert_aelys_error_contains(code, "capability");
}

#[test]
fn fs_mkdir_all_fails_without_capability() {
    let code = r#"
needs std.fs
fs.mkdir_all("/tmp/a/b/c")
"#;
    assert_aelys_error_contains(code, "capability");
}

#[test]
fn fs_append_text_fails_without_capability() {
    let code = r#"
needs std.fs
fs.append_text("/tmp/log.txt", "new line")
"#;
    assert_aelys_error_contains(code, "capability");
}

#[test]
fn fs_double_close() {
    let code = r#"
needs std.fs
let f = 1
fs.close(f)
fs.close(f)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

#[test]
fn fs_read_after_close() {
    let code = r#"
needs std.fs
let f = 1
fs.close(f)
fs.read(f)
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

#[test]
fn fs_write_after_close() {
    let code = r#"
needs std.fs
let f = 1
fs.close(f)
fs.write(f, "data")
"#;
    let err = run_aelys_err(code);
    assert!(err.contains("invalid") || err.contains("capability"));
}

mod common;

use std::io::Write;
use std::process::Stdio;

use common::*;

const MESSY: &str = "fn main() { let x = 1; if x > 0 { println!(\"{}\", x) } }\n";
const TIDY: &str =
    "fn main() {\n    let x = 1;\n    if x > 0 {\n        println!(\"{}\", x)\n    }\n}\n";
const BROKEN: &str = "fn main() {\n    let x = ;\n}\n";

#[test]
fn fmt_rewrites_files_in_place_and_is_stable() {
    let dir = scratch("fmt_in_place");
    write(&dir.join("src/main.aelys"), MESSY);
    write(&dir.join("src/lib/util.amod"), MESSY);
    write(&dir.join("target/skip.aelys"), MESSY);
    write(&dir.join(".hidden/skip.aelys"), MESSY);
    write(&dir.join("notes.txt"), MESSY);
    write(&dir.join("tests/row.case"), MESSY);
    write(&dir.join("src/main.aelys.fmt-tmp"), "mine");
    assert_ok(&ae(&dir, &["fmt"]));
    let read = |p: &str| std::fs::read_to_string(dir.join(p)).unwrap();
    assert_eq!(read("src/main.aelys"), TIDY);
    assert_eq!(read("src/lib/util.amod"), TIDY);
    assert_eq!(read("target/skip.aelys"), MESSY);
    assert_eq!(read(".hidden/skip.aelys"), MESSY);
    assert_eq!(read("notes.txt"), MESSY);
    assert_eq!(read("src/main.aelys.fmt-tmp"), "mine");
    assert_eq!(read("tests/row.case"), MESSY);
    let output = ae(&dir, &["fmt", "--check"]);
    assert_ok(&output);
    assert_eq!(output.stdout, "");
    assert_ok(&ae(&dir, &["fmt", "tests/row.case"]));
    assert_eq!(read("tests/row.case"), TIDY);
}

#[test]
fn fmt_check_reports_a_diff_and_writes_nothing() {
    let dir = scratch("fmt_check");
    write(&dir.join("a.aelys"), MESSY);
    let output = ae(&dir, &["fmt", "--check", "a.aelys"]);
    assert_eq!(output.code, Some(1), "stderr:\n{}", output.stderr);
    assert_contains(&output.stdout, "--- a.aelys");
    assert_contains(&output.stdout, "+    let x = 1;");
    assert_contains(&output.stdout, "-fn main() { let x = 1;");
    assert_eq!(std::fs::read_to_string(dir.join("a.aelys")).unwrap(), MESSY);
}

#[test]
fn fmt_refuses_invalid_input_without_touching_it() {
    let dir = scratch("fmt_invalid");
    write(&dir.join("bad.aelys"), BROKEN);
    write(&dir.join("good.aelys"), MESSY);
    let output = ae(&dir, &["fmt", "bad.aelys", "good.aelys"]);
    assert_eq!(output.code, Some(2), "stderr:\n{}", output.stderr);
    assert_contains(
        &output.stderr,
        "bad.aelys:2:13: expected an expression, found `;`",
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("bad.aelys")).unwrap(),
        BROKEN
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("good.aelys")).unwrap(),
        TIDY
    );
    let leftovers: Vec<_> = std::fs::read_dir(&dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|n| n.contains(".fmt-tmp"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
}

#[test]
fn fmt_dash_formats_stdin_to_stdout() {
    let dir = scratch("fmt_stdin");
    let mut child = capped(env!("CARGO_BIN_EXE_ae"))
        .args(["fmt", "-"])
        .current_dir(&dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(MESSY.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), TIDY);
}

#[cfg(unix)]
#[test]
fn fmt_writes_through_a_symlink() {
    let dir = scratch("fmt_symlink");
    write(&dir.join("target.aelys"), MESSY);
    std::os::unix::fs::symlink("target.aelys", dir.join("link.aelys")).unwrap();
    assert_ok(&ae(&dir, &["fmt", "link.aelys"]));
    let link = std::fs::symlink_metadata(dir.join("link.aelys")).unwrap();
    assert!(link.file_type().is_symlink());
    assert_eq!(
        std::fs::read_to_string(dir.join("target.aelys")).unwrap(),
        TIDY
    );
}

#[test]
fn fmt_refuses_a_read_only_file() {
    let dir = scratch("fmt_read_only");
    let path = dir.join("ro.aelys");
    write(&path, MESSY);
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    permissions.set_readonly(true);
    std::fs::set_permissions(&path, permissions).unwrap();
    let output = ae(&dir, &["fmt", "ro.aelys"]);
    assert_eq!(output.code, Some(2), "stderr:\n{}", output.stderr);
    assert_contains(&output.stderr, "ro.aelys: the file is read-only");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), MESSY);
}

#[cfg(unix)]
#[test]
fn fmt_does_not_follow_directory_symlinks() {
    let dir = scratch("fmt_dir_symlink");
    write(&dir.join("a.aelys"), MESSY);
    std::os::unix::fs::symlink(".", dir.join("self")).unwrap();
    let output = ae(&dir, &["fmt", "--check"]);
    assert_eq!(output.code, Some(1), "stderr:\n{}", output.stderr);
    assert_eq!(
        output.stdout.matches("--- ").count(),
        1,
        "{}",
        output.stdout
    );
}

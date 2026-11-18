use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

mod common;

const MAIN: &str = "fn main() {\n    println(\"ran\")\n}\n";
const NO_MAIN: &str = "pub fn f() -> i64 {\n    return 1\n}\n";
const WRONG: &str = "fn main() {\n    let x: string = 3\n}\n";

fn cli_bin() -> PathBuf {
    let _ = common::Cli::located();
    let exe = std::env::current_exe().unwrap();
    exe.parent().unwrap().parent().unwrap().join("aelys-cli")
}

fn run(bin: &Path, dir: &Path, args: &[&str]) -> Output {
    Command::new(bin)
        .args(args)
        .arg("-I")
        .arg(common::repo_root_from_manifest())
        .current_dir(dir)
        .output()
        .unwrap()
}

fn bare(bin: &Path, args: &[&str]) -> Output {
    Command::new(bin).args(args).output().unwrap()
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn objects_under(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            objects_under(&path, out);
        } else if path.extension().is_some_and(|e| e == "o" || e == "obj") {
            out.push(path);
        }
    }
}

fn no_object_left(dir: &Path) {
    let mut found = Vec::new();
    objects_under(dir, &mut found);
    assert!(found.is_empty(), "object files left behind: {found:?}");
}

fn staged(src: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::create_dir_all(dir.path().join("src")).unwrap();
    fs::create_dir_all(dir.path().join("out")).unwrap();
    fs::write(dir.path().join("src/prog.aelys"), src).unwrap();
    dir
}

#[test]
fn an_executable_at_dash_o_leaves_no_object() {
    let bin = cli_bin();
    let dir = staged(MAIN);
    let c = run(&bin, dir.path(), &["compile", "src/prog.aelys", "-o", "out/bin"]);
    assert!(c.status.success(), "compile failed:\n{}", text(&c.stderr));
    let r = Command::new(dir.path().join("out/bin")).output().unwrap();
    assert_eq!(text(&r.stdout), "ran\n");
    assert!(!dir.path().join("src/prog.o").exists());
    assert!(!dir.path().join("out/bin.o").exists());
    no_object_left(dir.path());
}

#[test]
fn an_executable_beside_the_source_leaves_no_object() {
    let bin = cli_bin();
    let dir = staged(MAIN);
    let c = run(&bin, dir.path(), &["compile", "src/prog.aelys"]);
    assert!(c.status.success(), "compile failed:\n{}", text(&c.stderr));
    assert!(dir.path().join("src/prog").is_file());
    assert!(!dir.path().join("src/prog.o").exists());
    no_object_left(dir.path());
}

#[test]
fn a_failed_link_leaves_no_object() {
    let bin = cli_bin();
    let dir = staged(MAIN);
    let c = run(
        &bin,
        dir.path(),
        &["compile", "src/prog.aelys", "-o", "out/bin", "-l", "aelys_e_absent"],
    );
    assert!(!c.status.success(), "a missing library must fail the link");
    no_object_left(dir.path());
}

#[test]
fn an_object_without_main_says_nothing_was_linked() {
    let bin = cli_bin();
    let dir = staged(NO_MAIN);
    let c = run(&bin, dir.path(), &["compile", "src/prog.aelys", "-o", "out/x"]);
    assert!(c.status.success(), "compile failed:\n{}", text(&c.stderr));
    assert!(dir.path().join("out/x").is_file());
    assert!(
        text(&c.stderr).contains("no `main`, so nothing was linked"),
        "stderr: {}",
        text(&c.stderr)
    );
}

#[test]
fn explain_and_version_write_to_stdout() {
    let bin = cli_bin();
    let e = bare(&bin, &["--explain", "E0106"]);
    assert!(e.status.success());
    assert!(text(&e.stdout).contains("not a bound"), "stdout: {}", text(&e.stdout));
    let u = bare(&bin, &["--explain", "E9999"]);
    assert_eq!(u.status.code(), Some(1));
    assert!(!u.stderr.is_empty());
    let v = bare(&bin, &["--version"]);
    assert!(v.status.success());
    assert!(!v.stdout.is_empty());
}

#[cfg(unix)]
#[test]
fn the_footer_names_the_invoked_binary() {
    let bin = cli_bin();
    let dir = staged(WRONG);
    let link = dir.path().join("aelysx");
    std::os::unix::fs::symlink(&bin, &link).unwrap();
    let c = run(&link, dir.path(), &["compile", "src/prog.aelys"]);
    assert!(!c.status.success());
    let err = text(&c.stderr);
    assert!(err.contains("try `aelysx --explain"), "stderr: {err}");
    assert!(!err.contains("try `aelys-cli"), "stderr: {err}");
}

#[test]
fn a_user_file_named_like_the_object_is_kept_byte_for_byte() {
    let bin = cli_bin();
    let dir = staged(MAIN);
    fs::create_dir_all(dir.path().join("out dir")).unwrap();
    let keep = dir.path().join("out dir/keep.o");
    fs::write(&keep, b"user data, not an object").unwrap();
    let c = run(&bin, dir.path(), &["compile", "src/prog.aelys", "-o", "out dir/keep"]);
    assert!(c.status.success(), "compile failed:\n{}", text(&c.stderr));
    let r = Command::new(dir.path().join("out dir/keep")).output().unwrap();
    assert_eq!(text(&r.stdout), "ran\n");
    assert_eq!(fs::read(&keep).unwrap(), b"user data, not an object");
}

#[test]
fn a_source_named_like_the_object_survives_its_build() {
    let bin = cli_bin();
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("p.o"), MAIN).unwrap();
    let c = run(&bin, dir.path(), &["compile", "p.o", "-o", "p"]);
    assert!(c.status.success(), "compile failed:\n{}", text(&c.stderr));
    assert_eq!(fs::read_to_string(dir.path().join("p.o")).unwrap(), MAIN);
    let r = Command::new(dir.path().join("p")).output().unwrap();
    assert_eq!(text(&r.stdout), "ran\n");
}

#[test]
fn an_output_that_names_the_source_is_refused() {
    let bin = cli_bin();
    for src in [MAIN, NO_MAIN] {
        let dir = staged(src);
        for extra in [&[][..], &["--emit-llvm-ir"][..]] {
            let mut args = vec!["compile", "src/prog.aelys", "-o", "src/prog.aelys"];
            args.extend_from_slice(extra);
            let c = run(&bin, dir.path(), &args);
            assert!(!c.status.success(), "{args:?} must be refused");
            assert!(
                text(&c.stderr).contains("is the source file itself"),
                "stderr: {}",
                text(&c.stderr)
            );
            assert_eq!(fs::read_to_string(dir.path().join("src/prog.aelys")).unwrap(), src);
        }
    }
}

mod common;

use std::path::Path;
use std::time::{Duration, SystemTime};

use common::*;

fn mtime(path: &Path) -> SystemTime {
    std::fs::metadata(path).unwrap().modified().unwrap()
}

fn compiles(output: &Output) -> bool {
    assert_ok(output);
    output.stderr.contains("   Compiling ")
}

#[test]
fn a_second_build_reuses_the_executable() {
    let dir = project("c_reuse", "", PRINTS_ONE);
    assert!(compiles(&ae(&dir, &["build"])));
    let exe = dir.join("target/debug/c_reuse");
    let before = mtime(&exe);

    let again = ae(&dir, &["build"]);
    assert!(!compiles(&again), "{}", again.stderr);
    assert_eq!(again.stderr.lines().count(), 1, "{}", again.stderr);
    assert_eq!(mtime(&exe), before);
}

#[test]
fn touching_a_source_does_not_recompile_but_editing_it_does() {
    let dir = project("c_edit", "", PRINTS_ONE);
    assert!(compiles(&ae(&dir, &["build"])));
    let main = dir.join("src/main.aelys");
    let later = SystemTime::now() + Duration::from_secs(5);
    std::fs::File::options()
        .write(true)
        .open(&main)
        .unwrap()
        .set_modified(later)
        .unwrap();
    assert!(!compiles(&ae(&dir, &["build"])));

    write(&main, PRINTS_TWO);
    let edited = ae(&dir, &["run"]);
    assert!(compiles(&edited));
    assert_eq!(edited.stdout, "2\n");
}

#[test]
fn a_new_source_file_or_a_removed_one_recompiles() {
    let dir = project("c_files", "", PRINTS_ONE);
    assert!(compiles(&ae(&dir, &["build"])));
    write(&dir.join("src/helper.aelys"), HELPER_MODULE);
    assert!(compiles(&ae(&dir, &["build"])));
    assert!(!compiles(&ae(&dir, &["build"])));
    std::fs::remove_file(dir.join("src/helper.aelys")).unwrap();
    assert!(compiles(&ae(&dir, &["build"])));
}

#[test]
fn a_deleted_executable_is_rebuilt() {
    let dir = project("c_deleted", "", PRINTS_ONE);
    assert!(compiles(&ae(&dir, &["build"])));
    std::fs::remove_file(dir.join("target/debug/c_deleted")).unwrap();
    assert!(compiles(&ae(&dir, &["build"])));
    assert!(dir.join("target/debug/c_deleted").is_file());
}

#[test]
fn profiles_are_cached_independently() {
    let dir = project("c_profiles", "", PRINTS_ONE);
    assert!(compiles(&ae(&dir, &["build"])));
    assert!(compiles(&ae(&dir, &["build", "--release"])));
    assert!(!compiles(&ae(&dir, &["build"])));
    assert!(!compiles(&ae(&dir, &["build", "--release"])));
}

#[test]
fn a_native_or_profile_change_recompiles() {
    let dir = project("c_native", "", PRINTS_ONE);
    assert!(compiles(&ae(&dir, &["build"])));
    let manifest = dir.join("Ae.toml");
    let base = std::fs::read_to_string(&manifest).unwrap();
    write(&manifest, &format!("{base}[native]\nlibs = [\"m\"]\n"));
    assert!(compiles(&ae(&dir, &["build"])));
    assert!(!compiles(&ae(&dir, &["build"])));
    write(
        &manifest,
        &format!("{base}[native]\nlibs = [\"m\"]\n[profile.dev]\nopt-level = 1\n"),
    );
    let output = ae(&dir, &["build"]);
    assert!(compiles(&output));
    assert_contains(&output.stderr, "`dev` profile [-O1, rc]");
}

#[test]
fn check_has_its_own_cache() {
    let dir = project("c_check", "", PRINTS_ONE);
    let first = ae(&dir, &["check"]);
    assert_ok(&first);
    assert_contains(&first.stderr, "    Checking c_check");
    let second = ae(&dir, &["check"]);
    assert_ok(&second);
    assert!(!second.stderr.contains("Checking"), "{}", second.stderr);
    assert!(compiles(&ae(&dir, &["build"])));
}

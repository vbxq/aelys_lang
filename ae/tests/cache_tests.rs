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
fn an_unrelated_file_does_not_recompile_but_a_removed_module_does() {
    let dir = project(
        "c_files",
        "",
        "needs helper\n\nfn main() -> i64 {\n    println(helper.helper())\n    return 0\n}\n",
    );
    write(&dir.join("src/helper.aelys"), HELPER_MODULE);
    assert!(compiles(&ae(&dir, &["build"])));
    write(&dir.join("src/notes.aelys"), PRINTS_TWO);
    assert!(!compiles(&ae(&dir, &["build"])));
    std::fs::remove_file(dir.join("src/helper.aelys")).unwrap();
    let removed = ae(&dir, &["build"]);
    assert_eq!(removed.code, Some(101), "{}", removed.stderr);
    assert_contains(&removed.stderr, "   Compiling c_files");
}

#[test]
fn a_module_behind_a_symlinked_directory_is_tracked() {
    let dir = project(
        "c_symlink",
        "",
        "needs shared.lib\n\nfn main() -> i64 {\n    println(lib.val())\n    return 0\n}\n",
    );
    let lib = dir.join("outside/lib.aelys");
    write(&lib, "pub fn val() -> i64 {\n    return 1\n}\n");
    std::os::unix::fs::symlink("../outside", dir.join("src/shared")).unwrap();
    let first = ae(&dir, &["run"]);
    assert!(compiles(&first));
    assert_eq!(first.stdout, "1\n");
    write(&lib, "pub fn val() -> i64 {\n    return 2\n}\n");
    let edited = ae(&dir, &["run"]);
    assert!(compiles(&edited));
    assert_eq!(edited.stdout, "2\n");
}

#[test]
fn a_codegen_knob_in_the_environment_recompiles() {
    let dir = project("c_env", "", PRINTS_ONE);
    assert!(compiles(&ae(&dir, &["build"])));
    let no_elision = [("AELYS_RC_ELISION", "0")];
    assert!(compiles(&ae_with(&dir, &["build"], &no_elision)));
    assert!(!compiles(&ae_with(&dir, &["build"], &no_elision)));
    let outlined = [("AELYS_OUTLINE_STR_COUNTS", "1")];
    assert!(compiles(&ae_with(&dir, &["build"], &outlined)));
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

#[test]
fn editing_a_module_inside_an_import_cycle_recompiles() {
    let dir = project(
        "c_cycle",
        "",
        "needs ev\n\nfn main() -> i64 {\n    println(ev.parity(7))\n    return 0\n}\n",
    );
    write(
        &dir.join("src/ev.aelys"),
        "needs od\n\npub fn is_even(n: i64) -> bool {\n    if n == 0 { return true }\n    return od.is_odd(n - 1)\n}\n\npub fn parity(n: i64) -> i64 {\n    if is_even(n) { return od.base() }\n    return od.base() + 1\n}\n",
    );
    let od = dir.join("src/od.aelys");
    let odd = |base: i64| {
        format!(
            "needs ev\n\npub fn is_odd(n: i64) -> bool {{\n    if n == 0 {{ return false }}\n    return ev.is_even(n - 1)\n}}\n\npub fn base() -> i64 {{\n    return {base}\n}}\n"
        )
    };
    write(&od, &odd(10));
    let first = ae(&dir, &["run"]);
    assert!(compiles(&first));
    assert_eq!(first.stdout, "11\n");
    assert!(!compiles(&ae(&dir, &["build"])));

    write(&od, &odd(20));
    let edited = ae(&dir, &["run"]);
    assert!(compiles(&edited));
    assert_eq!(edited.stdout, "21\n");
}

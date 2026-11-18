mod common;

use std::path::Path;

use common::*;

fn archive(dir: &Path) -> std::path::PathBuf {
    let lib = dir.join("vendor/lib");
    std::fs::create_dir_all(&lib).unwrap();
    let source = dir.join("vendor/aefoo.c");
    write(&source, AEFOO_C);
    let object = dir.join("vendor/aefoo.o");
    let status = capped("cc")
        .args(["-c", "-o"])
        .arg(&object)
        .arg(&source)
        .status()
        .unwrap();
    assert!(status.success());
    let archive = lib.join("libaefoo.a");
    let status = capped("ar")
        .arg("rcs")
        .arg(&archive)
        .arg(&object)
        .status()
        .unwrap();
    assert!(status.success());
    archive
}

#[test]
fn libm_from_native_libs_links_sqrt() {
    let dir = project("n_libm", "[native]\nlibs = [\"m\"]\n", SQRT_OF_TWO);
    let output = ae(&dir, &["run", "-q"]);
    assert_ok(&output);
    assert_eq!(output.stdout, "1.4142135623730951\n");
}

#[test]
fn a_native_library_that_does_not_exist_reaches_the_linker() {
    let dir = project(
        "n_missing",
        "[native]\nlibs = [\"ae_no_such_lib\"]\n",
        PRINTS_ONE,
    );
    let output = ae(&dir, &["build"]);
    assert_eq!(output.code, Some(101));
    assert_contains(&output.stderr, "cannot find -lae_no_such_lib");
    assert_contains(&output.stderr, "could not compile `n_missing`");
}

#[test]
fn search_and_libs_link_a_local_archive() {
    let dir = project(
        "n_search",
        "[native]\nlibs = [\"aefoo\"]\nsearch = [\"vendor/lib\"]\n",
        CALLS_AEFOO,
    );
    archive(&dir);
    let output = ae(&dir, &["run", "-q"]);
    assert_ok(&output);
    assert_eq!(output.stdout, "42\n");
}

#[test]
fn pkg_config_output_reaches_the_linker() {
    let dir = project("n_pc", "[native]\npkg-config = [\"aefoo\"]\n", CALLS_AEFOO);
    let archive = archive(&dir);
    let pc_dir = dir.join("pc");
    write(
        &pc_dir.join("aefoo.pc"),
        &format!(
            "Name: aefoo\nDescription: test archive\nVersion: 1.0\nLibs: {}\n",
            archive.display()
        ),
    );
    let pc_path = pc_dir.to_string_lossy().into_owned();
    let output = ae_with(&dir, &["run", "-q"], &[("PKG_CONFIG_PATH", &pc_path)]);
    assert_ok(&output);
    assert_eq!(output.stdout, "42\n");
}

#[test]
fn pkg_config_finds_sdl2() {
    let present = capped("pkg-config")
        .args(["--exists", "sdl2"])
        .status()
        .is_ok_and(|status| status.success());
    if !present {
        eprintln!("skipped: pkg-config does not know sdl2 on this machine");
        return;
    }
    let dir = project("n_sdl2", "[native]\npkg-config = [\"sdl2\"]\n", CALLS_SDL);
    let output = ae(&dir, &["run", "-q"]);
    assert_ok(&output);
    assert_eq!(output.stdout, "true\n");
}

#[test]
fn an_unknown_pkg_config_package_points_at_its_entry() {
    let dir = project(
        "n_nopc",
        "[native]\npkg-config = [\"aelys-nonexistent-pc\"]\n",
        PRINTS_ONE,
    );
    let output = ae(&dir, &["build"]);
    assert_eq!(output.code, Some(101));
    assert_contains(
        &output.stderr,
        "error: native library `aelys-nonexistent-pc` not found by pkg-config\n --> Ae.toml:5:15\n",
    );
    assert_contains(
        &output.stderr,
        "note: pkg-config said: Package aelys-nonexistent-pc was not found in the pkg-config search path.",
    );
}

#[test]
fn a_flag_in_native_libs_is_refused() {
    let dir = project("n_flag", "[native]\nlibs = [\"-lm\"]\n", PRINTS_ONE);
    let output = ae(&dir, &["build"]);
    assert_eq!(output.code, Some(101));
    assert_contains(&output.stderr, " --> Ae.toml:5:9\n");
    assert_contains(&output.stderr, "invalid native library name `-lm`");
}

#[test]
fn an_empty_or_flag_pkg_config_entry_is_refused() {
    for (name, entry) in [("n_pc_flag", "--static"), ("n_pc_empty", "")] {
        let native = format!("[native]\npkg-config = [\"{entry}\"]\n");
        let dir = project(name, &native, PRINTS_ONE);
        let output = ae(&dir, &["build"]);
        assert_eq!(output.code, Some(101), "stderr:\n{}", output.stderr);
        assert_contains(&output.stderr, " --> Ae.toml:5:15\n");
        let refusal = format!("invalid pkg-config package name `{entry}`");
        assert_contains(&output.stderr, &refusal);
    }
}

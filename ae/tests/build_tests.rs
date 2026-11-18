mod common;

use common::*;

#[test]
fn run_builds_then_prints_the_program_output() {
    let dir = project("b_sum", "", SUM_TO_TEN);
    let output = ae(&dir, &["run"]);
    assert_ok(&output);
    assert_eq!(output.stdout, "55\n");
    let lines: Vec<&str> = output.stderr.lines().collect();
    assert_eq!(lines.len(), 3, "{}", output.stderr);
    assert_eq!(
        lines[0],
        format!("   Compiling b_sum v0.1.0 ({})", dir.display())
    );
    assert!(
        lines[1].starts_with("    Finished `dev` profile [-O0, rc] target(s) in "),
        "{}",
        lines[1]
    );
    assert_eq!(lines[2], "     Running `target/debug/b_sum`");
}

#[test]
fn build_writes_an_executable_per_profile() {
    let dir = project("b_profiles", "", SUM_TO_TEN);
    assert_ok(&ae(&dir, &["build"]));
    let exe = dir.join("target/debug/b_profiles");
    let direct = capped(exe.to_str().unwrap()).output().unwrap();
    assert_eq!(String::from_utf8_lossy(&direct.stdout), "55\n");
    assert!(!dir.join("target/release/b_profiles").exists());

    let release = ae(&dir, &["build", "--release"]);
    assert_ok(&release);
    assert_contains(
        &release.stderr,
        "    Finished `release` profile [-O3, rc] target(s) in ",
    );
    assert!(dir.join("target/release/b_profiles").is_file());
}

#[test]
fn run_propagates_the_exit_code_of_the_program() {
    let dir = project("b_exit", "", EXITS_THREE);
    let output = ae(&dir, &["run"]);
    assert_eq!(output.code, Some(3), "stderr:\n{}", output.stderr);
    assert_contains(
        &output.stderr,
        "error: process didn't exit successfully: `target/debug/b_exit` (exit status: 3)",
    );
}

#[test]
fn a_program_killed_by_a_signal_makes_run_fail_with_101() {
    let dir = project("b_abort", "", ABORTS);
    let output = ae(&dir, &["run"]);
    assert_eq!(output.code, Some(101), "stderr:\n{}", output.stderr);
    assert_contains(&output.stderr, "(signal: 6, SIGABRT: process abort signal)");
}

#[test]
fn a_compile_error_shows_the_diagnostic_and_exits_101() {
    let dir = project("b_bad", "", TYPE_ERROR);
    let output = ae(&dir, &["build"]);
    assert_eq!(output.code, Some(101), "stderr:\n{}", output.stderr);
    assert_contains(
        &output.stderr,
        "error[E0301]: expected `i64`, found `string`",
    );
    assert_contains(&output.stderr, " --> src/main.aelys:2:5\n");
    assert_contains(
        &output.stderr,
        "error: could not compile `b_bad` (bin \"b_bad\") due to 1 previous error",
    );
    assert!(!output.stderr.contains("Finished"));
    assert!(!dir.join("target/debug/b_bad").exists());

    let check = ae(&dir, &["check"]);
    assert_eq!(check.code, Some(101));
    assert_contains(&check.stderr, "    Checking b_bad v0.1.0 (");
    assert_contains(&check.stderr, " --> src/main.aelys:2:5\n");
}

#[test]
fn check_accepts_a_correct_program_without_writing_an_executable() {
    let dir = project("b_check", "", SUM_TO_TEN);
    let output = ae(&dir, &["check"]);
    assert_ok(&output);
    assert_contains(&output.stderr, "    Checking b_check v0.1.0 (");
    assert!(!dir.join("target/debug/b_check").exists());
}

#[test]
fn a_bin_without_main_is_refused() {
    let dir = project("b_nomain", "", NO_MAIN);
    let output = ae(&dir, &["build"]);
    assert_eq!(output.code, Some(101));
    assert_contains(
        &output.stderr,
        "error: `main` function not found in `src/main.aelys`",
    );
    assert!(!dir.join("target/debug/b_nomain").exists());
}

#[test]
fn bins_under_src_bin_are_selected_by_name() {
    let dir = project("b_bins", "", PRINTS_ONE);
    write(&dir.join("src/bin/tool.aelys"), PRINTS_TWO);

    let ambiguous = ae(&dir, &["run"]);
    assert_eq!(ambiguous.code, Some(101));
    assert_contains(&ambiguous.stderr, "could not determine which binary to run");
    assert_contains(&ambiguous.stderr, "available binaries: b_bins, tool");

    let tool = ae(&dir, &["run", "-q", "--bin", "tool"]);
    assert_ok(&tool);
    assert_eq!(tool.stdout, "2\n");

    let typo = ae(&dir, &["run", "--bin", "tol"]);
    assert_eq!(typo.code, Some(101));
    assert_contains(&typo.stderr, "error: no bin target named `tol`");
    assert_contains(&typo.stderr, "a target with a similar name exists: `tool`");

    let manifest = std::fs::read_to_string(dir.join("Ae.toml")).unwrap();
    write(
        &dir.join("Ae.toml"),
        &manifest.replace(
            "version = \"0.1.0\"\n",
            "version = \"0.1.0\"\ndefault-run = \"tool\"\n",
        ),
    );
    let default = ae(&dir, &["run", "-q"]);
    assert_ok(&default);
    assert_eq!(default.stdout, "2\n");

    assert_ok(&ae(&dir, &["build"]));
    assert!(dir.join("target/debug/b_bins").is_file());
    assert!(dir.join("target/debug/tool").is_file());
}

#[test]
fn run_works_from_a_subdirectory() {
    let dir = project("b_subdir", "", PRINTS_ONE);
    let output = ae(&dir.join("src"), &["run", "-q"]);
    assert_ok(&output);
    assert_eq!(output.stdout, "1\n");
}

#[test]
fn a_bin_named_obj_builds() {
    let dir = project("obj", "", PRINTS_ONE);
    let output = ae(&dir, &["run"]);
    assert_ok(&output);
    assert_eq!(output.stdout, "1\n");
}

#[test]
fn a_bin_at_the_package_root_reads_only_its_modules() {
    use std::os::unix::fs::PermissionsExt;
    let dir = scratch("b_rootbin");
    write(
        &dir.join("Ae.toml"),
        "[package]\nname = \"b_rootbin\"\nversion = \"0.1.0\"\n\n[[bin]]\nname = \"b_rootbin\"\npath = \"main.aelys\"\n",
    );
    write(&dir.join("main.aelys"), PRINTS_ONE);
    let locked = dir.join("locked");
    std::fs::create_dir(&locked).unwrap();
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o000)).unwrap();
    let output = ae(&dir, &["run", "-q"]);
    std::fs::set_permissions(&locked, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_ok(&output);
    assert_eq!(output.stdout, "1\n");
}

#[test]
fn a_default_run_naming_no_bin_is_refused_by_build_and_check() {
    let dir = project("b_default", "default-run = \"tol\"\n", PRINTS_ONE);
    write(&dir.join("src/bin/tool.aelys"), PRINTS_TWO);
    for command in ["build", "check"] {
        let output = ae(&dir, &[command]);
        assert_eq!(output.code, Some(101), "stderr:\n{}", output.stderr);
        assert_contains(
            &output.stderr,
            "`default-run` names `tol`, which is not a bin target",
        );
        assert_contains(&output.stderr, " --> Ae.toml:4:15\n");
        assert_contains(
            &output.stderr,
            "a target with a similar name exists: `tool`",
        );
    }
}

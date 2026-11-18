mod common;

use common::*;

#[track_caller]
fn refused(name: &str, manifest: &str, expected: &[&str]) {
    let dir = scratch(name);
    write(&dir.join("Ae.toml"), manifest);
    write(&dir.join("src/main.aelys"), HELLO);
    let output = ae(&dir, &["build"]);
    assert_eq!(output.code, Some(101), "stderr:\n{}", output.stderr);
    for needle in expected {
        assert_contains(&output.stderr, needle);
    }
    assert!(!dir.join("target/debug").join(name).exists());
}

#[test]
fn an_unknown_key_points_at_it_and_suggests_the_known_one() {
    refused(
        "m_unknown",
        "[package]\nname = \"m_unknown\"\nversion = \"0.1.0\"\n\n[dependancies]\nx = 1\n",
        &[
            "error: failed to parse manifest `",
            " --> Ae.toml:5:2\n",
            "5 | [dependancies]\n",
            "^^^^^^^^^^^^ unknown field `dependancies`",
            "help: did you mean `dependencies`?",
            "note: expected one of `package`, `dependencies`, `dev-dependencies`, `native`",
        ],
    );
    refused(
        "m_unknown_package_key",
        "[package]\nnmae = \"x\"\nname = \"m_unknown_package_key\"\nversion = \"0.1.0\"\n",
        &[
            " --> Ae.toml:2:1\n",
            "unknown field `nmae`",
            "help: did you mean `name`?",
        ],
    );
}

#[test]
fn invalid_toml_is_reported_with_a_pointer() {
    refused(
        "m_syntax",
        "[package\nname = \"m_syntax\"\n",
        &[
            " --> Ae.toml:1:9\n",
            "1 | [package\n",
            "^ invalid table header",
        ],
    );
}

#[test]
fn a_version_that_is_not_semver_is_refused() {
    refused(
        "m_semver",
        "[package]\nname = \"m_semver\"\nversion = \"0.1\"\n",
        &[
            " --> Ae.toml:3:11\n",
            "^^^^^ `0.1` is not a semver version",
            "help: write three components, e.g. `0.1.0`",
        ],
    );
}

#[test]
fn a_reserved_or_malformed_package_name_is_refused() {
    let base = scratch("m_names");
    write(
        &base.join("Ae.toml"),
        "[package]\nname = \"std\"\nversion = \"0.1.0\"\n",
    );
    write(&base.join("src/main.aelys"), HELLO);
    let output = ae(&base, &["build"]);
    assert_eq!(output.code, Some(101));
    assert_contains(&output.stderr, " --> Ae.toml:2:8\n");
    assert_contains(
        &output.stderr,
        "invalid package name `std`: `std` is a reserved name",
    );

    write(
        &base.join("Ae.toml"),
        "[package]\nname = \"a.b\"\nversion = \"0.1.0\"\n",
    );
    let output = ae(&base, &["build"]);
    assert_eq!(output.code, Some(101));
    assert_contains(
        &output.stderr,
        "the character `.` is not allowed in a package name",
    );
}

#[test]
fn profile_values_are_checked() {
    refused(
        "m_opt",
        "[package]\nname = \"m_opt\"\nversion = \"0.1.0\"\n[profile.dev]\nopt-level = 5\n",
        &[
            " --> Ae.toml:5:13\n",
            "opt-level must be 0, 1, 2 or 3, not 5",
        ],
    );
    refused(
        "m_runtime",
        "[package]\nname = \"m_runtime\"\nversion = \"0.1.0\"\n[profile.release]\nruntime = \"gc\"\n",
        &[
            " --> Ae.toml:5:11\n",
            "unknown runtime `gc`",
            "expected `rc`, `rc+cycles` or `leak`",
        ],
    );
    refused(
        "m_profile",
        "[package]\nname = \"m_profile\"\nversion = \"0.1.0\"\n[profile.bench]\n",
        &["unknown field `bench`", "expected one of `dev`, `release`"],
    );
}

#[test]
fn dependencies_are_refused_by_name_until_supported() {
    refused(
        "m_deps",
        "[package]\nname = \"m_deps\"\nversion = \"0.1.0\"\n\n[dependencies]\nutil = { path = \"../util\" }\n",
        &[
            " --> Ae.toml:5:1\n",
            "`dependencies` is not supported by this version of ae",
        ],
    );
    let dir = project("m_empty_deps", "[dependencies]\n", PRINTS_ONE);
    assert_ok(&ae(&dir, &["build"]));
}

#[test]
fn a_missing_manifest_names_the_directory() {
    let dir = scratch("m_missing");
    let output = ae(&dir, &["build"]);
    assert_eq!(output.code, Some(101));
    assert_contains(
        &output.stderr,
        &format!(
            "error: could not find `Ae.toml` in `{}` or any parent directory",
            dir.display()
        ),
    );

    let elsewhere = project("m_elsewhere", "", PRINTS_ONE);
    let manifest = elsewhere.join("Ae.toml");
    let output = ae(
        &dir,
        &["run", "-q", "--manifest-path", manifest.to_str().unwrap()],
    );
    assert_ok(&output);
    assert_eq!(output.stdout, "1\n");
}

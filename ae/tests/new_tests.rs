mod common;

use common::*;

fn tree(dir: &std::path::Path) -> Vec<String> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        for entry in std::fs::read_dir(next).unwrap() {
            let path = entry.unwrap().path();
            let relative = path
                .strip_prefix(dir)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            if relative == ".git" {
                found.push(".git/".to_string());
            } else if path.is_dir() {
                found.push(format!("{relative}/"));
                pending.push(path);
            } else {
                found.push(relative);
            }
        }
    }
    found.sort();
    found
}

#[test]
fn new_creates_the_package_tree_and_the_template_runs() {
    let base = scratch("new_tree");
    let ceiling = base.parent().unwrap().to_string_lossy().into_owned();
    let output = ae_with(
        &base,
        &["new", "hello"],
        &[("GIT_CEILING_DIRECTORIES", &ceiling)],
    );
    assert_ok(&output);
    assert_eq!(
        output.stderr,
        "    Creating binary (application) `hello` package\n"
    );

    let dir = base.join("hello");
    assert_eq!(
        tree(&dir),
        [".git/", ".gitignore", "Ae.toml", "src/", "src/main.aelys"]
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("Ae.toml")).unwrap(),
        "[package]\nname = \"hello\"\nversion = \"0.1.0\"\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join(".gitignore")).unwrap(),
        "/target\n"
    );
    assert_eq!(
        std::fs::read_to_string(dir.join("src/main.aelys")).unwrap(),
        HELLO
    );

    let run = ae(&dir, &["run", "-q"]);
    assert_ok(&run);
    assert_eq!(run.stdout, "Hello, world!\n");
}

#[test]
fn new_inside_a_repository_or_with_vcs_none_does_not_init_git() {
    let base = scratch("new_novcs");
    let ceiling = base.parent().unwrap().to_string_lossy().into_owned();
    let output = ae_with(
        &base,
        &["new", "plain", "--vcs", "none"],
        &[("GIT_CEILING_DIRECTORIES", &ceiling)],
    );
    assert_ok(&output);
    assert!(!base.join("plain/.git").exists());

    let nested = ae(&base, &["new", "nested"]);
    assert_ok(&nested);
    assert!(
        !base.join("nested/.git").exists(),
        "the worktree already is a repository"
    );
}

#[test]
fn new_refuses_an_existing_destination() {
    let base = scratch("new_existing");
    std::fs::create_dir(base.join("taken")).unwrap();
    let output = ae(&base, &["new", "taken"]);
    assert_eq!(output.code, Some(101));
    assert_contains(
        &output.stderr,
        &format!(
            "error: destination `{}` already exists",
            base.join("taken").display()
        ),
    );
    assert!(
        std::fs::read_dir(base.join("taken"))
            .unwrap()
            .next()
            .is_none()
    );
}

#[test]
fn new_refuses_an_invalid_name_unless_one_is_given() {
    let base = scratch("new_badname");
    let output = ae(&base, &["new", "2game"]);
    assert_eq!(output.code, Some(101));
    assert_contains(
        &output.stderr,
        "error: invalid package name `2game`: a package name cannot start with a digit",
    );
    assert_contains(&output.stderr, "help: pass `--name <name>`");
    assert!(!base.join("2game").exists());

    let renamed = ae(&base, &["new", "2game", "--name", "game2"]);
    assert_ok(&renamed);
    assert_contains(
        &std::fs::read_to_string(base.join("2game/Ae.toml")).unwrap(),
        "name = \"game2\"",
    );

    let reserved = ae(&base, &["new", "std"]);
    assert_eq!(reserved.code, Some(101));
    assert_contains(&reserved.stderr, "`std` is a reserved name");
}

#[test]
fn help_lists_the_subcommands_and_usage_errors_exit_two() {
    let base = scratch("new_help");
    let help = ae(&base, &["help"]);
    assert_ok(&help);
    for command in ["new", "build", "check", "run"] {
        assert_contains(&help.stdout, &format!("\n  {command} "));
    }
    assert_eq!(ae(&base, &[]).code, Some(2));
    assert_eq!(ae(&base, &["frobnicate"]).code, Some(2));
}

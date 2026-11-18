use std::path::Path;
use std::process::Command;

use crate::error::{Result, error, io_error, with_help};
use crate::manifest::check_name;
use crate::shell::Shell;
use crate::templates;

#[derive(Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Vcs {
    Git,
    None,
}

pub fn run(shell: &Shell, path: &Path, name: Option<String>, vcs: Vcs) -> Result<()> {
    let destination = std::path::absolute(path)
        .map_err(|err| io_error(format_args!("invalid path `{}`", path.display()), err))?;
    if destination.exists() {
        return Err(with_help(
            error(format!(
                "destination `{}` already exists",
                destination.display()
            )),
            "choose another path",
        )
        .into());
    }
    let name = match name {
        Some(name) => name,
        None => destination
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| {
                with_help(
                    error(format!(
                        "cannot infer a package name from `{}`",
                        path.display()
                    )),
                    "pass `--name <name>`",
                )
            })?
            .to_string(),
    };
    if let Err(reason) = check_name(&name) {
        return Err(with_help(
            error(format!("invalid package name `{name}`: {reason}")),
            "pass `--name <name>` to name the package differently from its directory",
        )
        .into());
    }
    let init_git = vcs == Vcs::Git && !inside_git_work_tree(&destination)?;

    shell.status("Creating", format!("binary (application) `{name}` package"));
    let write = |relative: &str, contents: &str| {
        let file = destination.join(relative);
        std::fs::create_dir_all(file.parent().expect("a template file lives in a directory"))
            .and_then(|()| std::fs::write(&file, contents))
            .map_err(|err| io_error(format_args!("cannot write `{}`", file.display()), err))
    };
    write("Ae.toml", &templates::manifest(&name))?;
    write("src/main.aelys", templates::BIN_MAIN)?;
    write(".gitignore", templates::GITIGNORE)?;
    if init_git {
        git(&["init", "--quiet"], &destination)?;
    }
    Ok(())
}

fn inside_git_work_tree(destination: &Path) -> Result<bool> {
    let existing = destination
        .ancestors()
        .find(|dir| dir.is_dir())
        .unwrap_or(Path::new("/"));
    let output = Command::new("git")
        .args(["rev-parse", "--is-inside-work-tree"])
        .current_dir(existing)
        .output()
        .map_err(|err| {
            with_help(
                error(format!("failed to run `git`: {err}")),
                "pass `--vcs none` to create the package without a repository",
            )
        })?;
    Ok(output.status.success())
}

fn git(args: &[&str], dir: &Path) -> Result<()> {
    let output = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .map_err(|err| error(format!("failed to run `git`: {err}")))?;
    if output.status.success() {
        return Ok(());
    }
    let mut diagnostic = error(format!(
        "`git {}` failed in `{}`",
        args.join(" "),
        dir.display()
    ));
    let said = String::from_utf8_lossy(&output.stderr);
    if let Some(first) = said.lines().find(|line| !line.trim().is_empty()) {
        diagnostic.add_note(first.trim().to_string());
    }
    Err(diagnostic.into())
}

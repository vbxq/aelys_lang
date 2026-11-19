use std::path::Path;
use std::process::{Command, ExitStatus};

use crate::compile::{Mode, compile};
use crate::error::{Error, FAILURE, Result, error, io_error};
use crate::shell::Shell;
use crate::workspace::Package;

pub fn run(
    shell: &Shell,
    package: &Package,
    bin: Option<&str>,
    release: bool,
    args: &[String],
    invoked_from: &Path,
) -> Result<()> {
    let bin = package.runnable(bin)?;
    let executable = compile(shell, package, &[bin], release, Mode::Build)?
        .pop()
        .expect("one bin was built");
    shell.status("Running", format!("`{}`", executable.display()));
    let status = Command::new(package.root.join(&executable))
        .args(args)
        .current_dir(invoked_from)
        .status()
        .map_err(|err| {
            io_error(
                format_args!("could not execute `{}`", executable.display()),
                err,
            )
        })?;
    if status.success() {
        return Ok(());
    }
    let diagnostic = error(format!(
        "process didn't exit successfully: `{}` ({})",
        executable.display(),
        describe(status)
    ));
    Err(Error {
        diagnostics: vec![diagnostic],
        code: status
            .code()
            .and_then(|code| u8::try_from(code).ok())
            .unwrap_or(FAILURE),
    })
}

#[cfg(unix)]
fn describe(status: ExitStatus) -> String {
    use std::os::unix::process::ExitStatusExt;
    if let Some(code) = status.code() {
        return format!("exit status: {code}");
    }
    let signal = status.signal().unwrap_or(0);
    let name = match signal {
        2 => "SIGINT: interrupt",
        4 => "SIGILL: illegal instruction",
        6 => "SIGABRT: process abort signal",
        7 => "SIGBUS: access to undefined memory",
        8 => "SIGFPE: erroneous arithmetic operation",
        9 => "SIGKILL: kill",
        11 => "SIGSEGV: invalid memory reference",
        15 => "SIGTERM: termination signal",
        _ => return format!("signal: {signal}"),
    };
    format!("signal: {signal}, {name}")
}

#[cfg(not(unix))]
fn describe(status: ExitStatus) -> String {
    format!("exit status: {}", status.code().unwrap_or(-1))
}

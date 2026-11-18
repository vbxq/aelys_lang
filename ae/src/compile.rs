use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Instant, UNIX_EPOCH};

use aelys_common::{Diagnostic, Severity, WarningConfig, format_warnings};
use aelys_driver::{
    BuildRequest, CompiledArtifact, LinkRequirement, RuntimeVariant, SourceOptions,
};

use crate::error::{Error, FAILURE, Result, error, io_error, with_help};
use crate::fingerprint::{self, Key};
use crate::manifest::{Profile, runtime_name};
use crate::native;
use crate::shell::Shell;
use crate::workspace::{Bin, Package};

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Build,
    Check,
}

pub fn compile(
    shell: &Shell,
    package: &Package,
    bins: &[&Bin],
    release: bool,
    mode: Mode,
) -> Result<Vec<PathBuf>> {
    let started = Instant::now();
    let (profile_name, dir_name, profile) = if release {
        ("release", "release", package.manifest.release)
    } else {
        ("dev", "debug", package.manifest.dev)
    };
    let out_dir = Path::new("target").join(dir_name);
    let _lock = lock_build_dir(shell)?;

    let (link, core_lib) = match mode {
        Mode::Build => (
            native::link_requirement(&package.manifest, &package.root)?,
            Some(runtime_archive(profile.runtime)?),
        ),
        Mode::Check => (LinkRequirement::default(), None),
    };

    let mut announced = false;
    let mut executables = Vec::new();
    for bin in bins {
        let artifact = (mode == Mode::Build).then(|| out_dir.join(&bin.name));
        let key = Key {
            compiler: compiler_identity(),
            std: format!("embedded:{}", aelys_driver::embedded_std_hash()),
            core_lib: core_lib.as_deref().map(file_identity).unwrap_or_default(),
            options: options(bin, mode, profile, &link),
            artifact: artifact
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned()),
        };
        let suffix = if mode == Mode::Check { "-check" } else { "" };
        let record = out_dir.join(".fingerprint").join(format!(
            "{}-bin-{}{suffix}.toml",
            package.manifest.name, bin.name
        ));
        let dir = match bin.path.parent() {
            Some(dir) if !dir.as_os_str().is_empty() => dir,
            _ => Path::new("."),
        };
        let inputs = fingerprint::scan(dir, &[Path::new("Ae.toml"), &bin.path]).map_err(|err| {
            io_error(
                format_args!("cannot read the sources of `{}`", bin.name),
                err,
            )
        })?;
        if let Some(artifact) = &artifact {
            executables.push(artifact.clone());
        }
        if fingerprint::is_fresh(&record, &key, &inputs) {
            continue;
        }
        if !announced {
            let verb = if mode == Mode::Check {
                "Checking"
            } else {
                "Compiling"
            };
            shell.status(
                verb,
                format!(
                    "{} v{} ({})",
                    package.manifest.name,
                    package.manifest.version,
                    package.root.display()
                ),
            );
            announced = true;
        }
        let pending = fingerprint::snapshot(key, &inputs).map_err(|err| {
            io_error(
                format_args!("cannot read the sources of `{}`", bin.name),
                err,
            )
        })?;
        match (&artifact, &core_lib) {
            (Some(artifact), Some(core_lib)) => build_one(
                &package.manifest.name,
                bin,
                &out_dir,
                artifact,
                core_lib,
                profile,
                &link,
            )?,
            _ => check_one(&package.manifest.name, bin)?,
        }
        pending
            .commit(&record)
            .map_err(|err| io_error(format_args!("cannot write `{}`", record.display()), err))?;
    }

    shell.status(
        "Finished",
        format!(
            "`{profile_name}` profile [-O{}, {}] target(s) in {:.2}s",
            profile.opt_level.numeric(),
            runtime_name(profile.runtime),
            started.elapsed().as_secs_f64()
        ),
    );
    Ok(executables)
}

fn build_one(
    package: &str,
    bin: &Bin,
    out_dir: &Path,
    artifact: &Path,
    core_lib: &Path,
    profile: Profile,
    link: &LinkRequirement,
) -> Result<()> {
    let obj_dir = out_dir.join("obj");
    std::fs::create_dir_all(&obj_dir)
        .map_err(|err| io_error(format_args!("cannot create `{}`", obj_dir.display()), err))?;
    let object = obj_dir.join(format!("{}.o", bin.name));
    let request = BuildRequest {
        root: &bin.path,
        sources: &SourceOptions::default(),
        opt_level: profile.opt_level,
        runtime: profile.runtime,
        link,
        core_lib,
        output: artifact,
        object: &object,
    };
    match aelys_driver::build(&request) {
        Ok((warnings, CompiledArtifact::Executable(_))) => {
            report_warnings(warnings);
            Ok(())
        }
        Ok((_, CompiledArtifact::Object(path) | CompiledArtifact::LlvmIr(path))) => {
            let _ = std::fs::remove_file(path);
            Err(with_help(
                error(format!(
                    "`main` function not found in `{}`",
                    bin.path.display()
                )),
                "a bin target needs a `main` function",
            )
            .into())
        }
        Err(err) => Err(could_not_compile(package, bin, err.to_diagnostics())),
    }
}

fn check_one(package: &str, bin: &Bin) -> Result<()> {
    match aelys_driver::check_file(&bin.path, &SourceOptions::default()) {
        Ok(warnings) => {
            report_warnings(warnings);
            Ok(())
        }
        Err(err) => Err(could_not_compile(package, bin, err.to_diagnostics())),
    }
}

fn report_warnings(warnings: Vec<aelys_common::Warning>) {
    let config = WarningConfig::default();
    let shown: Vec<_> = warnings
        .into_iter()
        .filter(|warning| config.is_enabled(&warning.kind))
        .collect();
    if !shown.is_empty() {
        eprintln!("{}", format_warnings(&shown));
    }
}

fn could_not_compile(package: &str, bin: &Bin, mut diagnostics: Vec<Diagnostic>) -> Error {
    let count = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == Severity::Error)
        .count();
    let plural = if count == 1 { "" } else { "s" };
    diagnostics.push(Diagnostic::new(
        Severity::Error,
        format!(
            "could not compile `{package}` (bin \"{}\") due to {count} previous error{plural}",
            bin.name
        ),
    ));
    Error {
        diagnostics,
        code: FAILURE,
    }
}

fn options(bin: &Bin, mode: Mode, profile: Profile, link: &LinkRequirement) -> String {
    let root = bin.path.display();
    match mode {
        Mode::Check => format!("kind=bin mode=check root={root}"),
        Mode::Build => format!(
            "kind=bin mode=build root={root} -O{} runtime={} link={}",
            profile.opt_level.numeric(),
            runtime_name(profile.runtime),
            native::describe(link)
        ),
    }
}

fn file_identity(path: &Path) -> String {
    let (size, mtime) = std::fs::metadata(path)
        .map(|metadata| {
            let mtime = metadata
                .modified()
                .ok()
                .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
                .map_or(0, |since| since.as_nanos());
            (metadata.len(), mtime)
        })
        .unwrap_or_default();
    format!("{} {size} {mtime}", path.display())
}

fn compiler_identity() -> String {
    let exe = std::env::current_exe().unwrap_or_default();
    format!("ae {} {}", env!("CARGO_PKG_VERSION"), file_identity(&exe))
}

fn runtime_archive(runtime: RuntimeVariant) -> Result<PathBuf> {
    if let Some(explicit) = std::env::var_os("AELYS_CORE_LIB") {
        let path = PathBuf::from(explicit);
        if path.is_file() {
            return Ok(path);
        }
        return Err(error(format!(
            "AELYS_CORE_LIB points to a missing file: {}",
            path.display()
        ))
        .into());
    }
    let name = if cfg!(windows) {
        format!("aelys-core-{}.lib", runtime.lib_suffix())
    } else {
        format!("libaelys-core-{}.a", runtime.lib_suffix())
    };
    let path = Path::new(env!("AE_CORE_LIB_DIR")).join(name);
    if path.is_file() {
        return Ok(path);
    }
    Err(with_help(
        error(format!(
            "the Aelys runtime archive `{}` is missing",
            path.display()
        )),
        "rebuild ae with `cargo build -p ae`, or set AELYS_CORE_LIB",
    )
    .into())
}

fn lock_build_dir(shell: &Shell) -> Result<File> {
    std::fs::create_dir_all("target").map_err(|err| io_error("cannot create `target`", err))?;
    let file = File::options()
        .create(true)
        .write(true)
        .truncate(false)
        .open("target/.ae-lock")
        .map_err(|err| io_error("cannot open `target/.ae-lock`", err))?;
    match file.try_lock() {
        Ok(()) => {}
        Err(std::fs::TryLockError::WouldBlock) => {
            shell.status("Blocking", "waiting for file lock on build directory");
            file.lock()
                .map_err(|err| io_error("cannot lock `target/.ae-lock`", err))?;
        }
        Err(std::fs::TryLockError::Error(err)) => {
            return Err(io_error("cannot lock `target/.ae-lock`", err));
        }
    }
    Ok(file)
}

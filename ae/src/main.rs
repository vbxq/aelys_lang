mod commands;
mod compile;
mod error;
mod fingerprint;
mod manifest;
mod native;
mod shell;
mod templates;
mod workspace;

use std::path::PathBuf;
use std::process::ExitCode;

use aelys_common::ColorConfig;
use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::commands::new::Vcs;
use crate::compile::Mode;
use crate::error::{Result, io_error};
use crate::shell::Shell;

#[derive(Parser)]
#[command(
    name = "ae",
    version,
    about = "The Aelys build tool",
    arg_required_else_help = true
)]
struct Cli {
    /// Coloring of status lines and diagnostics
    #[arg(long, global = true, value_enum, default_value_t = ColorWhen::Auto, value_name = "WHEN")]
    color: ColorWhen,
    /// Print no status lines
    #[arg(short, long, global = true)]
    quiet: bool,
    /// Path to the Ae.toml to use instead of searching upward from the current directory
    #[arg(long, global = true, value_name = "PATH")]
    manifest_path: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Clone, Copy, ValueEnum)]
enum ColorWhen {
    Auto,
    Always,
    Never,
}

#[derive(Subcommand)]
enum Command {
    /// Create a new package at <PATH>
    New {
        path: PathBuf,
        /// Package name, by default the last component of <PATH>
        #[arg(long)]
        name: Option<String>,
        /// Initialize a version control repository
        #[arg(long, value_enum, default_value_t = Vcs::Git)]
        vcs: Vcs,
    },
    /// Compile the package's binaries
    Build(Target),
    /// Check the package for errors without generating code
    Check(Target),
    /// Build and run a binary of the package
    Run {
        #[command(flatten)]
        target: Target,
        /// Arguments passed to the program
        #[arg(last = true)]
        args: Vec<String>,
    },
}

#[derive(Args)]
struct Target {
    /// Use the release profile
    #[arg(long)]
    release: bool,
    /// Select one binary by name
    #[arg(long, value_name = "NAME")]
    bin: Option<String>,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let color = match cli.color {
        ColorWhen::Auto => ColorConfig::auto(),
        ColorWhen::Always => ColorConfig::always(),
        ColorWhen::Never => ColorConfig::never(),
    };
    let shell = Shell::new(color, cli.quiet);
    match execute(&shell, cli.command, cli.manifest_path) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            for diagnostic in &error.diagnostics {
                shell.diagnostic(diagnostic);
            }
            ExitCode::from(error.code)
        }
    }
}

fn execute(shell: &Shell, command: Command, manifest_path: Option<PathBuf>) -> Result<()> {
    let (target, mode, run_args) = match command {
        Command::New { path, name, vcs } => return commands::new::run(shell, &path, name, vcs),
        Command::Build(target) => (target, Mode::Build, None),
        Command::Check(target) => (target, Mode::Check, None),
        Command::Run { target, args } => (target, Mode::Build, Some(args)),
    };
    let invoked_from = std::env::current_dir()
        .map_err(|err| io_error("cannot read the current directory", err))?;
    let package = workspace::load(&workspace::locate(manifest_path.as_deref())?)?;
    std::env::set_current_dir(&package.root).map_err(|err| {
        io_error(
            format_args!("cannot enter `{}`", package.root.display()),
            err,
        )
    })?;
    let bin = target.bin.as_deref();
    match run_args {
        Some(args) => {
            commands::run::run(shell, &package, bin, target.release, &args, &invoked_from)
        }
        None => commands::build::run(shell, &package, bin, target.release, mode),
    }
}

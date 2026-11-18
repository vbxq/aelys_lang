use crate::compile::{Mode, compile};
use crate::error::Result;
use crate::shell::Shell;
use crate::workspace::Package;

pub fn run(
    shell: &Shell,
    package: &Package,
    bin: Option<&str>,
    release: bool,
    mode: Mode,
) -> Result<()> {
    let bins = package.select(bin)?;
    compile(shell, package, &bins, release, mode).map(drop)
}

use std::path::Path;
use std::process::Command;

use aelys_driver::LinkRequirement;

use crate::error::{Result, error, with_help};
use crate::manifest::Manifest;

pub fn link_requirement(manifest: &Manifest, root: &Path) -> Result<LinkRequirement> {
    let native = &manifest.native;
    let mut link = LinkRequirement {
        search_paths: native.search.iter().map(|dir| root.join(dir)).collect(),
        libraries: native.libs.clone(),
        args: Vec::new(),
    };
    for entry in &native.pkg_config {
        let output = Command::new("pkg-config")
            .arg("--libs")
            .arg(entry.get_ref())
            .env("LC_ALL", "C")
            .output()
            .map_err(|err| {
                with_help(
                    error(format!("failed to run `pkg-config`: {err}")),
                    "`[native] pkg-config` in Ae.toml needs pkg-config installed",
                )
            })?;
        if !output.status.success() {
            let mut diagnostic = manifest.origin.error_at(
                entry.span(),
                format!(
                    "native library `{}` not found by pkg-config",
                    entry.get_ref()
                ),
            );
            let said = String::from_utf8_lossy(&output.stderr);
            if let Some(first) = said.lines().map(str::trim).find(|line| !line.is_empty()) {
                diagnostic.add_note(format!("pkg-config said: {first}"));
            }
            diagnostic.add_help(
                "install the library's development package, or point PKG_CONFIG_PATH at its .pc file",
            );
            return Err(diagnostic.into());
        }
        for arg in String::from_utf8_lossy(&output.stdout).split_whitespace() {
            if let Some(dir) = arg.strip_prefix("-L") {
                link.search_paths.push(dir.into());
            } else if let Some(name) = arg.strip_prefix("-l") {
                link.libraries.push(name.to_string());
            } else {
                link.args.push(arg.to_string());
            }
        }
    }
    Ok(link)
}

pub fn describe(link: &LinkRequirement) -> String {
    let search = link
        .search_paths
        .iter()
        .map(|dir| format!("-L{}", dir.display()));
    let libraries = link.libraries.iter().map(|name| format!("-l{name}"));
    search
        .chain(libraries)
        .chain(link.args.iter().cloned())
        .collect::<Vec<_>>()
        .join(" ")
}

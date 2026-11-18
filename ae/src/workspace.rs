use std::path::{Path, PathBuf};

use aelys_common::Diagnostic;

use crate::error::{Result, closest, error, io_error, with_help};
use crate::manifest::{Manifest, check_name};

pub struct Package {
    pub root: PathBuf,
    pub manifest: Manifest,
    pub bins: Vec<Bin>,
}

pub struct Bin {
    pub name: String,
    pub path: PathBuf,
}

pub fn locate(cwd: &Path, explicit: Option<&Path>) -> Result<PathBuf> {
    if let Some(path) = explicit {
        let path = cwd.join(path);
        if !path.is_file() {
            return Err(error(format!("manifest path `{}` does not exist", path.display())).into());
        }
        return Ok(path);
    }
    for dir in cwd.ancestors() {
        let candidate = dir.join("Ae.toml");
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(with_help(
        error(format!(
            "could not find `Ae.toml` in `{}` or any parent directory",
            cwd.display()
        )),
        "create a package with `ae new <path>`, or pass `--manifest-path`",
    )
    .into())
}

pub fn load(manifest_path: &Path) -> Result<Package> {
    let text = std::fs::read_to_string(manifest_path).map_err(|err| {
        io_error(
            format_args!("cannot read `{}`", manifest_path.display()),
            err,
        )
    })?;
    let manifest = Manifest::parse(&text, manifest_path)?;
    let root = manifest_path
        .parent()
        .expect("a manifest path names a file")
        .to_path_buf();
    let bins = discover_bins(&root, &manifest)?;
    if let Some(default) = &manifest.default_run {
        let name = default.get_ref();
        if !bins.iter().any(|bin| bin.name == *name) {
            let mut diagnostic = manifest.origin.error_at(
                default.span(),
                format!("`default-run` names `{name}`, which is not a bin target"),
            );
            suggest(&mut diagnostic, name, &bins);
            return Err(diagnostic.into());
        }
    }
    Ok(Package {
        root,
        manifest,
        bins,
    })
}

fn discover_bins(root: &Path, manifest: &Manifest) -> Result<Vec<Bin>> {
    let mut bins = Vec::new();
    for decl in &manifest.bins {
        let name = decl.name.get_ref();
        let path = match &decl.path {
            Some(path) => PathBuf::from(path),
            None if *name == manifest.name => PathBuf::from("src/main.aelys"),
            None => Path::new("src/bin").join(format!("{name}.aelys")),
        };
        if !root.join(&path).is_file() {
            return Err(manifest
                .origin
                .error_at(
                    decl.name.span(),
                    format!("can't find `{}` for bin `{name}`", path.display()),
                )
                .into());
        }
        if bins.iter().any(|bin: &Bin| bin.name == *name) {
            return Err(manifest
                .origin
                .error_at(
                    decl.name.span(),
                    format!("two bin targets are named `{name}`"),
                )
                .into());
        }
        bins.push(Bin {
            name: name.clone(),
            path,
        });
    }

    let mut found = Vec::new();
    if root.join("src/main.aelys").is_file() {
        found.push((manifest.name.clone(), PathBuf::from("src/main.aelys")));
    }
    if let Ok(entries) = std::fs::read_dir(root.join("src/bin")) {
        let mut extra: Vec<PathBuf> = entries
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.is_file() && path.extension().is_some_and(|ext| ext == "aelys"))
            .collect();
        extra.sort();
        for path in extra {
            let stem = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            let relative = Path::new("src/bin").join(path.file_name().unwrap_or_default());
            if let Err(reason) = check_name(&stem, "target") {
                return Err(error(format!(
                    "`{}` cannot be a bin target: {}",
                    relative.display(),
                    reason
                ))
                .into());
            }
            found.push((stem, relative));
        }
    }
    for (name, path) in found {
        let declared = bins.iter().any(|bin| bin.name == name || bin.path == path);
        if !declared {
            bins.push(Bin { name, path });
        }
    }
    Ok(bins)
}

impl Package {
    pub fn select(&self, name: Option<&str>) -> Result<Vec<&Bin>> {
        if let Some(name) = name {
            return Ok(vec![self.bin(name)?]);
        }
        if self.bins.is_empty() {
            return Err(with_help(
                error(format!(
                    "package `{}` has no bin target",
                    self.manifest.name
                )),
                "create `src/main.aelys`, or declare a `[[bin]]` in Ae.toml",
            )
            .into());
        }
        Ok(self.bins.iter().collect())
    }

    pub fn runnable(&self, name: Option<&str>) -> Result<&Bin> {
        if let Some(name) = name {
            return self.bin(name);
        }
        if let Some(default) = &self.manifest.default_run {
            return self.bin(default.get_ref());
        }
        match self.select(None)?.as_slice() {
            [only] => Ok(only),
            many => {
                let names: Vec<&str> = many.iter().map(|bin| bin.name.as_str()).collect();
                let mut diagnostic = error(
                    "`ae run` could not determine which binary to run; use `--bin`, or set \
                     `default-run` in Ae.toml",
                );
                diagnostic.add_note(format!("available binaries: {}", names.join(", ")));
                Err(diagnostic.into())
            }
        }
    }

    fn bin(&self, name: &str) -> Result<&Bin> {
        if let Some(bin) = self.bins.iter().find(|bin| bin.name == name) {
            return Ok(bin);
        }
        let mut diagnostic = error(format!("no bin target named `{name}`"));
        suggest(&mut diagnostic, name, &self.bins);
        Err(diagnostic.into())
    }
}

fn suggest(diagnostic: &mut Diagnostic, name: &str, bins: &[Bin]) {
    if let Some(similar) = closest(name, bins.iter().map(|bin| bin.name.as_str())) {
        diagnostic.add_help(format!("a target with a similar name exists: `{similar}`"));
    }
}

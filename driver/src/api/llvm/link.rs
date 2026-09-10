use std::path::{Path, PathBuf};

use super::runtime::RuntimeVariant;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LinkRequirement {
    pub search_paths: Vec<PathBuf>,
    pub libraries: Vec<String>,
}

impl LinkRequirement {
    pub fn is_empty(&self) -> bool {
        self.search_paths.is_empty() && self.libraries.is_empty()
    }
}

pub(super) fn link_native_executable(
    object_path: &Path,
    exe_path: &Path,
    core_lib: &Path,
    runtime: RuntimeVariant,
    link: &LinkRequirement,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        let _ = runtime;
        link_windows(object_path, exe_path, core_lib, link)
    }
    #[cfg(not(windows))]
    {
        link_unix(object_path, exe_path, core_lib, runtime, link)
    }
}

#[cfg(windows)]
fn link_windows(
    object_path: &Path,
    exe_path: &Path,
    core_lib: &Path,
    link: &LinkRequirement,
) -> Result<(), String> {
    let obj = object_path.to_string_lossy().to_string();
    let exe = exe_path.to_string_lossy().to_string();
    let lib = core_lib.to_string_lossy().to_string();

    let mut link_args = vec![
        "/NOLOGO".to_string(),
        "/SUBSYSTEM:CONSOLE".to_string(),
        format!("/OUT:{}", exe),
        obj,
        lib,
        "msvcrt.lib".to_string(),
        "kernel32.lib".to_string(),
    ];
    for dir in &link.search_paths {
        link_args.push(format!("/LIBPATH:{}", dir.to_string_lossy()));
    }
    for name in &link.libraries {
        link_args.push(format!("{}.lib", name));
    }

    let mut errors = Vec::new();
    for linker in windows_linkers() {
        match super::run_process_c_locale(&linker, &link_args) {
            Ok(()) => return Ok(()),
            Err(err) => errors.push(err),
        }
    }

    Err(errors.join("\n"))
}

#[cfg(not(windows))]
fn link_unix(
    object_path: &Path,
    exe_path: &Path,
    core_lib: &Path,
    runtime: RuntimeVariant,
    link: &LinkRequirement,
) -> Result<(), String> {
    let obj = object_path.to_string_lossy().to_string();
    let exe = exe_path.to_string_lossy().to_string();
    let lib_dir = core_lib
        .parent()
        .ok_or_else(|| format!("invalid aelys-core path: {}", core_lib.display()))?;

    let mut args = vec![
        "-o".to_string(),
        exe,
        obj,
        format!("-L{}", lib_dir.to_string_lossy()),
        format!("-laelys-core-{}", runtime.lib_suffix()),
    ];
    for dir in &link.search_paths {
        args.push(format!("-L{}", dir.to_string_lossy()));
    }
    for name in &link.libraries {
        args.push(format!("-l{}", name));
    }
    super::run_process_c_locale("cc", &args)
}

#[cfg(windows)]
fn windows_linkers() -> Vec<String> {
    let mut linkers = Vec::new();

    for prefix in llvm_sys_18x_prefixes() {
        let candidate = prefix.join("bin").join("lld-link.exe");
        if candidate.is_file() {
            let linker = candidate.to_string_lossy().to_string();
            if !linkers.iter().any(|existing| existing == &linker) {
                linkers.push(linker);
            }
        }
    }

    let default_lld = PathBuf::from(r"C:\llvm\bin\lld-link.exe");
    if default_lld.is_file() {
        let lld = default_lld.to_string_lossy().to_string();
        if !linkers.iter().any(|existing| existing == &lld) {
            linkers.push(lld);
        }
    }

    linkers.push("lld-link".to_string());
    linkers.push("link".to_string());
    linkers
}

#[cfg(windows)]
fn llvm_sys_18x_prefixes() -> Vec<PathBuf> {
    let mut entries = std::env::vars()
        .filter(|(key, value)| {
            key.starts_with("LLVM_SYS_18") && key.ends_with("_PREFIX") && !value.is_empty()
        })
        .collect::<Vec<_>>();
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
        .into_iter()
        .map(|(_, value)| PathBuf::from(value))
        .collect()
}

fn defined_symbols(path: &Path) -> Option<std::collections::HashSet<String>> {
    let out = super::capture_process(
        "nm",
        &[
            "--extern-only".to_string(),
            "--defined-only".to_string(),
            path.to_string_lossy().to_string(),
        ],
    )?;
    let mut found = std::collections::HashSet::new();
    for line in out.lines() {
        let fields = line.split_whitespace().collect::<Vec<_>>();
        if fields.len() != 3 {
            continue;
        }
        if fields[1].len() == 1 && fields[1] != "U" {
            found.insert(fields[2].to_string());
        }
    }
    if found.is_empty() { None } else { Some(found) }
}

pub(super) fn linked_library_claims_runtime_symbol(
    exe_path: &Path,
    core_lib: &Path,
    link: &LinkRequirement,
) -> Option<String> {
    if link.libraries.is_empty() {
        return None;
    }
    let in_exe = defined_symbols(exe_path)?;
    let in_core = defined_symbols(core_lib)?;
    aelys_air::symbols::RUNTIME_RESERVED_SYMBOLS
        .iter()
        .find(|symbol| in_exe.contains(**symbol) && !in_core.contains(**symbol))
        .map(|symbol| (*symbol).to_string())
}

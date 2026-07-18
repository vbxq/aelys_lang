use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

use super::runtime::RuntimeVariant;

pub(super) fn link_native_executable(
    object_path: &Path,
    exe_path: &Path,
    core_lib: &Path,
    runtime: RuntimeVariant,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        // the variant is already baked into `core_lib`, windows links the path directly
        let _ = runtime;
        link_windows(object_path, exe_path, core_lib)
    }
    #[cfg(not(windows))]
    {
        link_unix(object_path, exe_path, core_lib, runtime)
    }
}

#[cfg(windows)]
fn link_windows(object_path: &Path, exe_path: &Path, core_lib: &Path) -> Result<(), String> {
    let obj = object_path.to_string_lossy().to_string();
    let exe = exe_path.to_string_lossy().to_string();
    let lib = core_lib.to_string_lossy().to_string();

    let link_args = vec![
        "/NOLOGO".to_string(),
        "/SUBSYSTEM:CONSOLE".to_string(),
        format!("/OUT:{}", exe),
        obj,
        lib,
        "msvcrt.lib".to_string(),
        "kernel32.lib".to_string(),
    ];

    let mut errors = Vec::new();
    for linker in windows_linkers() {
        match super::run_process(&linker, &link_args) {
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
) -> Result<(), String> {
    let obj = object_path.to_string_lossy().to_string();
    let exe = exe_path.to_string_lossy().to_string();
    let lib_dir = core_lib
        .parent()
        .ok_or_else(|| format!("invalid aelys-core path: {}", core_lib.display()))?;

    let args = vec![
        "-o".to_string(),
        exe,
        obj,
        format!("-L{}", lib_dir.to_string_lossy()),
        format!("-laelys-core-{}", runtime.lib_suffix()),
    ];
    super::run_process("cc", &args)
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
